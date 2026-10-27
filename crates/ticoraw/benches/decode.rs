//! Wall-clock benchmarks (criterion): the full TicoRAW decode of a synthetic 24.5 MP frame
//! at the HE and HE* rates, each decoder stage on its own, and optionally a real NEF.
//!
//! ```sh
//! cargo bench -p ticoraw --bench decode                          # all cores
//! RAYON_NUM_THREADS=1 cargo bench -p ticoraw --bench decode      # one core, same code
//! cargo bench -p ticoraw --bench decode --no-default-features    # no rayon at all
//! TICORAW_BENCH_NEF=/path/to/file.NEF cargo bench -p ticoraw --bench decode -- nef
//! ```
//!
//! Throughput is reported in photosites (CFA samples) per second, so criterion's
//! "Melem/s" reads as MP/s.
//!
//! The stage benches mirror `decode_ticoraw`, but entropy decoding and the horizontal
//! wavelet run slice by slice and row by row on one thread; the vertical wavelet,
//! Star-Tetrix and the curve use the crate's own parallelism when `parallel` is enabled.

mod support;

use std::hint::black_box;
use std::sync::OnceLock;
use std::time::Duration;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use support::{Frame, Stages};

/// Nikon Z6III FX: the most common HE/HE* frame size (docs/compat.md).
const WIDTH: usize = 6064;
const HEIGHT: usize = 4040;
/// Constant strip rates in bits per photosite (docs/compat.md).
const HE_BPP: f64 = 3.0;
const HE_STAR_BPP: f64 = 5.0;

fn frame(bpp: f64) -> &'static Frame {
    static HE: OnceLock<Frame> = OnceLock::new();
    static HE_STAR: OnceLock<Frame> = OnceLock::new();
    let cell = if bpp < 4.0 { &HE } else { &HE_STAR };
    cell.get_or_init(|| {
        let f = support::encode(WIDTH, HEIGHT, bpp);
        let image = ticoraw::decode_ticoraw(&f.codestream).expect("synthetic frame decodes");
        assert_eq!(
            Stages::new(&f).all(&f),
            image.pixels,
            "stages match decode_ticoraw"
        );
        assert!(f.mean_abs_error < 256.0, "the stream carries the scene");
        eprintln!(
            "synthetic {}x{}: {:.3} bpp, {} bytes, precinct Q {}..{}, mean error {:.2} codes",
            f.width,
            f.height,
            f.bpp(),
            f.codestream.len(),
            f.q_range.0,
            f.q_range.1,
            f.mean_abs_error
        );
        f
    })
}

/// Photosites per frame, known without encoding one (setup is lazy, so a filtered run such
/// as `-- decode_nef` does not pay for encoding frames it never decodes).
const PHOTOSITES: u64 = (WIDTH * HEIGHT) as u64;

fn full_decode(c: &mut Criterion) {
    let mut g = c.benchmark_group("decode_ticoraw");
    g.sample_size(20).measurement_time(Duration::from_secs(8));
    g.throughput(Throughput::Elements(PHOTOSITES));
    for (name, bpp) in [("he_3bpp", HE_BPP), ("he_star_5bpp", HE_STAR_BPP)] {
        g.bench_function(name, |b| {
            let f = frame(bpp);
            b.iter(|| ticoraw::decode_ticoraw(black_box(&f.codestream)).expect("decodes"));
        });
    }
    g.finish();
}

/// The HE* frame and the input of every stage.
struct StageInputs {
    frame: &'static Frame,
    stages: Stages,
    coefficients: [Vec<i32>; 4],
    rows: [Vec<i32>; 4],
    wavelet: [Vec<i32>; 4],
    colour: [Vec<i32>; 4],
}

fn stage_inputs() -> &'static StageInputs {
    static INPUTS: OnceLock<StageInputs> = OnceLock::new();
    INPUTS.get_or_init(|| {
        let frame = frame(HE_STAR_BPP);
        let stages = Stages::new(frame);
        let coefficients = stages.entropy(frame);
        let mut rows = coefficients.clone();
        stages.horizontal(&mut rows);
        let mut wavelet = rows.clone();
        stages.vertical(&mut wavelet);
        let mut colour = wavelet.clone();
        stages.star_tetrix(&mut colour);
        StageInputs {
            frame,
            stages,
            coefficients,
            rows,
            wavelet,
            colour,
        }
    })
}

fn stages(c: &mut Criterion) {
    let mut g = c.benchmark_group("stage_he_star");
    g.sample_size(20).measurement_time(Duration::from_secs(6));
    g.throughput(Throughput::Elements(PHOTOSITES));
    g.bench_function("entropy", |b| {
        let i = stage_inputs();
        b.iter(|| i.stages.entropy(black_box(i.frame)));
    });
    g.bench_function("idwt_horizontal", |b| {
        let i = stage_inputs();
        b.iter_batched_ref(
            || i.coefficients.clone(),
            |p| i.stages.horizontal(p),
            BatchSize::LargeInput,
        );
    });
    g.bench_function("idwt_vertical", |b| {
        let i = stage_inputs();
        b.iter_batched_ref(
            || i.rows.clone(),
            |p| i.stages.vertical(p),
            BatchSize::LargeInput,
        );
    });
    g.bench_function("star_tetrix", |b| {
        let i = stage_inputs();
        b.iter_batched_ref(
            || i.wavelet.clone(),
            |p| i.stages.star_tetrix(p),
            BatchSize::LargeInput,
        );
    });
    g.bench_function("curve", |b| {
        let i = stage_inputs();
        b.iter(|| i.stages.curve(black_box(&i.colour)));
    });
    g.finish();
}

/// Decodes the NEF named by `TICORAW_BENCH_NEF`, container parsing included.
fn real_nef(c: &mut Criterion) {
    let Some(path) = std::env::var_os("TICORAW_BENCH_NEF") else {
        return;
    };
    let nef = std::fs::read(&path).expect("TICORAW_BENCH_NEF is readable");
    let raw = ticoraw::decode_nef(&nef).expect("TICORAW_BENCH_NEF is an HE/HE* NEF");
    let mut g = c.benchmark_group("decode_nef");
    g.sample_size(20).measurement_time(Duration::from_secs(8));
    g.throughput(Throughput::Elements(
        (raw.image.width * raw.image.height) as u64,
    ));
    g.bench_function(raw.model.as_str(), |b| {
        b.iter(|| ticoraw::decode_nef(black_box(&nef)).expect("decodes"));
    });
    g.finish();
}

criterion_group!(benches, full_decode, stages, real_nef);
criterion_main!(benches);
