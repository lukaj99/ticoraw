# My editor can't open my HE files

You took photos with a Nikon camera, and your photo editor shows an error, a blank
thumbnail, or nothing at all. If your camera was set to **High Efficiency** raw, this
page is for you. It explains what is going on and how ticoraw fixes it.

## What HE and HE\* are

Nikon raw files end in `.NEF`. On recent Nikon Z cameras you can choose how the raw data
inside a NEF is compressed. The menu is usually called **RAW recording** (under **Image
quality** in the photo shooting menu). The choices are:

| Setting | What it means | Opens in most editors? |
|---|---|---|
| Lossless compressed | Nikon's classic raw format | Yes |
| **High efficiency\*** (HE\*) | A newer, much smaller format with very high quality | Often not |
| **High efficiency** (HE) | The same format, even smaller | Often not |

HE and HE\* files are smaller because they use a newer kind of compression, called
TicoRAW, made by a company named intoPIX. It is based on a standard called JPEG XS. Until
recently there was no public description of how to read it, so many editors could not
add support.

These cameras can write HE and HE\*: the Nikon **Z 9, Z 8, Z f, Z6III, Z 5 II, Z 50 II**
and **ZR**.

## Which apps open HE files

Some apps open HE files directly. Others need the files converted first.

**Open HE files directly** (in recent versions):

- Adobe Lightroom, Lightroom Classic and Camera Raw (and Adobe DNG Converter);
- Nikon's own NX Studio;
- some other commercial editors, for example recent versions of DxO PhotoLab.

**Need conversion first** (at the time of writing, October 2026):

- darktable, which lists "Nikon high efficiency NEFs" as not supported on its
  [camera support page](https://www.darktable.org/resources/camera-support/);
- RawTherapee, digiKam and other apps built on the LibRaw library, whose 0.22 release
  supports these cameras with standard compression only;
- older versions of the apps in the first list.

Support changes over time, so check your app's release notes too. If your app opens your
files already, you do not need ticoraw.

## How ticoraw helps

ticoraw reads your HE and HE\* NEFs and writes a **DNG** file next to each one. DNG is an
open raw format that almost every raw editor opens, including all of the apps above.

The DNG is still a raw file, not a JPEG. It holds exactly the same raw values Adobe's
converters would produce, so you keep all of your editing latitude. It also keeps your
camera settings, date, lens, GPS position and a preview.

Your NEF files are never changed. ticoraw only reads them.

## Is my file HE?

If you are not sure which setting you used, ticoraw can tell you once it is installed:

```sh
ticoraw scan ~/Pictures/MyCard --recursive
```

It prints one line per NEF. Lines that start with `ticoraw`, followed by `HE` or `HE*`,
are High Efficiency files: those need converting. Lines that start with `classic` are
Nikon's older formats, which your editor already opens.

## Three steps

1. [Install ticoraw](install.md). It takes about ten minutes the first time.
2. [Convert your first card](first-conversion.md) with one command.
3. Open the DNG files in your editor.

If something goes wrong, the [troubleshooting page](troubleshooting.md) covers the
common problems.

> [!NOTE]
> Today ticoraw is installed by building it from its source code, which the install guide
> explains step by step. Ready-made downloads and a converter in your web browser are
> planned, pending a patent-licensing review. [Patents and downloads](../patents.md) says
> why.
