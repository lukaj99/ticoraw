# Frequently asked questions

## Using ticoraw

### Why are there no downloads? Can I just get an app?

Not yet, but they are planned. HE files use a patented compression format (JPEG XS /
TicoRAW), whose decoders are licensed per copy. Before the project hands out compiled copies
of a decoder, that licensing is being reviewed. Until then, ticoraw is installed by building
it from source, which the [install guide](photographers/install.md) walks you through.
[Patents and downloads](patents.md) explains the details and lists what is coming.

### Is it safe for my files?

Yes. ticoraw is careful with your photos:

- **It never modifies your NEFs.** It opens them for reading only.
- **It writes atomically.** Each DNG goes to a temporary file first, and only a complete
  file is renamed into place. A failure part-way leaves no half-written DNG behind (a hard
  kill or power loss can leave a hidden `.tmp` file next to it, which is safe to delete).
- **It never overwrites by accident.** An existing DNG is kept unless you pass `--force`,
  and ticoraw refuses to write over its own input file.
- **The decoder is robust.** It is tested against corrupted and hostile files and fuzzed,
  and it returns an error rather than crashing.

As with any tool, keep a backup of your originals.

### Does it upload my photos anywhere?

No. ticoraw runs entirely on your computer. It makes no network connections, uploads
nothing and collects no telemetry. The planned browser converter is designed the same way:
files are processed inside your browser and never leave your device.

### Which cameras are supported?

Every Nikon body known to write HE or HE\*: the Z 9, Z 8, Z f, Z6III, Z 5 II, Z 50 II and ZR.
Each is checked bit-exact against Adobe on public sample files; see the
[compatibility matrix](compat.md). If you have a camera or firmware that is not listed, a
[sample](contributing/samples.md) would help.

### What about my older, classic NEFs?

ticoraw only converts HE and HE\* files and skips the others. Classic NEFs (lossless
compressed, lossy or uncompressed) already open in almost every raw editor, so they need no
conversion.

### Does it work with Nikon N-RAW video?

It has not been tested. N-RAW frames reportedly use the same codestream, but ticoraw reads
NEF still images only.

## About the DNG files

### Why DNG?

DNG ("Digital Negative") is an openly documented raw format, published by Adobe, that
almost every raw editor opens. A DNG is still a raw file: it holds the sensor's mosaic, not
a finished picture, so you keep all of your editing latitude. Converting to TIFF or JPEG
instead would "bake in" a look and lose that.

### Are the DNGs the same as Adobe's?

The raw data is: every photosite has exactly the value Adobe DNG Converter and Lightroom
produce. That is checked on public samples from every supported body. The black and white
levels, colour matrix, camera name, `BaselineExposure`, orientation and crop match Adobe's
too.

A few things differ. Adobe's own camera profile data (`ForwardMatrix`, hue and look tables)
is Adobe's and is not copied, along with a few other optional tags such as `NoiseProfile`
and lens-correction opcodes. Adobe software applies its own profile anyway, because the DNG
names the camera the way Adobe does.

### Why do my photos look different in my editor than on the camera?

Each raw editor renders raw data with its own colour profile and tone curve, so the same
raw file looks different in darktable, Lightroom or the camera's JPEG. That is normal for
every raw file, and it is not caused by the conversion. The DNG's raw values are identical
to Adobe's.

### My HLG photos look dark

Photos taken in Nikon's **HLG tone mode** keep extra headroom for bright highlights, so
their raw data is darker than a normal photo's. Adobe compensates with a DNG tag called `BaselineExposure`, which
tells the editor to brighten the image. For HLG frames it adds one stop (1 EV) on top of the
camera's usual value. ticoraw writes exactly the same value as Adobe.

Editors that apply `BaselineExposure` (Adobe's do) show these photos at the intended
brightness. An editor that ignores the tag shows them about one stop too dark. In that case,
raise the exposure by about +1 EV in your editor, or save it as a preset for your HLG
photos.

### Why are DNGs bigger than my NEFs?

HE and HE\* are **lossy** formats: the camera keeps very high quality but throws some data
away to make the files small. A DNG stores the decoded raw values with **lossless**
compression, which cannot be as small. Typically a DNG is 1.2 to 2.2 times the size of its
NEF. With `--uncompressed`, it takes 2 bytes per photosite: about 49 MB for a 24 MP frame.
See [File sizes](converting.md#file-sizes).

### Can I delete the NEFs after converting?

You can, but we suggest keeping them. The DNG holds the same raw data and metadata, but the
NEF is the original your camera wrote. Keeping it costs disk space and keeps every option
open.

### Does ticoraw use my sidecar edits?

Only if you ask. With `--with-sidecar`, a Lightroom-style `DSC_0001.xmp` next to the NEF
becomes the DNG's XMP. See [Sidecars](converting.md#sidecars).

## About the project

### How is this different from Adobe DNG Converter?

Adobe DNG Converter is free, works well, and is the reference ticoraw is checked against.
ticoraw adds things it doesn't do: it is open source, runs on Linux as well as macOS and
Windows, is easy to script, and is a library you can build into other software.

### Is ticoraw free?

Yes. It is open source under the MIT or Apache-2.0 licence, at your choice. Those licences
cover the code's copyright; they cannot grant patent rights. See
[Patents and downloads](patents.md).

### How do I know the decoder is correct?

It is checked against Adobe's output on public sample files from every supported camera,
with zero differing photosites, and that check runs in CI on every change. The
[conformance kit](conformance.md) lets anyone repeat it, and `ticoraw verify` compares a
decode against any DNG you have.

### How can I help?

Try it on your photos and tell us how it went, share a [camera sample](contributing/samples.md),
or improve the documentation or code. [Contributing](contributing/index.md) explains how.
