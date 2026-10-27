//! Instruction-count benchmarks (gungraun, formerly iai-callgrind) for the CI regression
//! gate. Callgrind counts the instructions each decoder stage executes, which, unlike wall
//! time, does not depend on the machine's load, so a shared CI runner can compare a pull
//! request against its base and fail on a small regression.
//!
//! Linux only (Valgrind). Build without the `parallel` feature so rayon's scheduling does
//! not add noise to the counts, and install the runner at the locked gungraun version:
//!
//! ```sh
//! cargo install gungraun-runner --version <gungraun version in Cargo.lock>
//! cargo bench -p ticoraw --no-default-features --bench instructions
//! ```
//!
//! The frame is 1024x776 photosites at the HE* rate: 13 slices with a short last one, like
//! camera frames, so every band, slice and edge path runs, yet small enough that the whole
//! run (Valgrind also executes each benchmark's setup, which encodes the frame) takes about
//! a minute.

#[cfg(target_os = "linux")]
mod support;

#[cfg(target_os = "linux")]
mod gate {
    // gungraun moves each setup's value into its benchmark.
    #![allow(clippy::needless_pass_by_value)]

    use std::hint::black_box;

    use gungraun::{
        Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group,
    };

    use super::support::{self, Frame, Stages};

    const WIDTH: usize = 1024;
    const HEIGHT: usize = 776;
    const HE_STAR_BPP: f64 = 5.0;

    fn frame() -> Frame {
        support::encode(WIDTH, HEIGHT, HE_STAR_BPP)
    }

    /// Planes after entropy decoding (plus `extra` further stages) and their stage state.
    fn planes(extra: usize) -> (Stages, [Vec<i32>; 4]) {
        let f = frame();
        let s = Stages::new(&f);
        let mut p = s.entropy(&f);
        if extra >= 1 {
            s.horizontal(&mut p);
        }
        if extra >= 2 {
            s.vertical(&mut p);
        }
        if extra >= 3 {
            s.star_tetrix(&mut p);
        }
        (s, p)
    }

    fn after_entropy() -> (Stages, [Vec<i32>; 4]) {
        planes(0)
    }

    fn after_horizontal() -> (Stages, [Vec<i32>; 4]) {
        planes(1)
    }

    fn after_vertical() -> (Stages, [Vec<i32>; 4]) {
        planes(2)
    }

    fn after_star_tetrix() -> (Stages, [Vec<i32>; 4]) {
        planes(3)
    }

    fn with_stages() -> (Stages, Frame) {
        let f = frame();
        (Stages::new(&f), f)
    }

    #[library_benchmark]
    #[bench::he_star(setup = frame)]
    fn decode_ticoraw(f: Frame) -> ticoraw::CfaImage {
        black_box(ticoraw::decode_ticoraw(black_box(&f.codestream)).expect("decodes"))
    }

    #[library_benchmark]
    #[bench::he_star(setup = with_stages)]
    fn entropy((s, f): (Stages, Frame)) -> [Vec<i32>; 4] {
        black_box(s.entropy(black_box(&f)))
    }

    #[library_benchmark]
    #[bench::he_star(setup = after_entropy)]
    fn idwt_horizontal((s, mut p): (Stages, [Vec<i32>; 4])) -> [Vec<i32>; 4] {
        s.horizontal(black_box(&mut p));
        black_box(p)
    }

    #[library_benchmark]
    #[bench::he_star(setup = after_horizontal)]
    fn idwt_vertical((s, mut p): (Stages, [Vec<i32>; 4])) -> [Vec<i32>; 4] {
        s.vertical(black_box(&mut p));
        black_box(p)
    }

    #[library_benchmark]
    #[bench::he_star(setup = after_vertical)]
    fn star_tetrix((s, mut p): (Stages, [Vec<i32>; 4])) -> [Vec<i32>; 4] {
        s.star_tetrix(black_box(&mut p));
        black_box(p)
    }

    #[library_benchmark]
    #[bench::he_star(setup = after_star_tetrix)]
    fn curve((s, p): (Stages, [Vec<i32>; 4])) -> Vec<u16> {
        black_box(s.curve(black_box(&p)))
    }

    library_benchmark_group!(
        name = decoder;
        benchmarks = decode_ticoraw, entropy, idwt_horizontal, idwt_vertical, star_tetrix, curve
    );

    /// Fails the run when any benchmark executes over 5% more instructions than the
    /// baseline it is compared with (CI passes the same limit on the command line).
    pub fn config() -> LibraryBenchmarkConfig {
        let mut config = LibraryBenchmarkConfig::default();
        config.tool(Callgrind::default().soft_limits([(EventKind::Ir, 5.0)]));
        config
    }
}

#[cfg(target_os = "linux")]
use gate::{config, decoder};

#[cfg(target_os = "linux")]
gungraun::main!(
    config = config();
    library_benchmark_groups = decoder
);

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("instruction-count benchmarks need Valgrind and run on Linux only");
}
