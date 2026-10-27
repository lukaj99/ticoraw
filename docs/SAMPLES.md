# Contributing a camera sample

One raw file from your camera can make `ticoraw` work, provably, for everyone who owns the
same body. Each new sample is converted with Adobe DNG Converter, compared with
`ticoraw`'s output at every photosite and, once it matches, becomes a permanent test case
listed in [compat.md](compat.md). That is how every body in the README's status table got
there.

This guide takes about fifteen minutes and needs no programming. The short version:

1. Read [what a NEF file contains](#what-a-nef-file-contains) and decide whether you are
   happy to publish it.
2. [Set up the camera](#1-set-up-the-camera) and
   [take the picture](#2-take-the-picture).
3. [Copy the file](#3-copy-the-file-without-changing-it) straight from the card.
4. [Upload it to raw.pixls.us](#4-upload-it-to-rawpixlsus) under the CC0 licence.
5. [Open a camera support issue](#5-tell-us-about-it) with the link.

Please **do not** attach raw files to issues or pull requests, or send them to this
repository in any other way. Samples live in a public archive, not here.

## Which samples help most

We are most interested in files that are not yet listed in [compat.md](compat.md):

- a **body** that is not in the table at all, for example a new Nikon model that offers
  High Efficiency;
- an **HE** or **HE\*** file from a body where we only have the other mode (for example
  HE from a ZR);
- a **firmware version** newer than the one in the table, especially a major update;
- an **image area** we have not seen for that body: FX, DX, 1:1, 16:9 or others;
- **HLG tone mode** files, and any setting you suspect changes the raw file.

A pair of files, one HE and one HE\*, of the same scene is ideal.

## What a NEF file contains

A NEF is more than the picture. The camera writes metadata alongside it, and **anything
you upload to a public archive is public forever**. Here is what to expect:

| Data | Where in the file | Can you avoid it? |
|---|---|---|
| **Camera body serial number** | Nikon MakerNote (and on some bodies also the EXIF block) | **No.** Every NEF has it |
| Lens identification, possibly including its serial number | EXIF and MakerNote | No |
| Shutter count (how many pictures the camera has taken) | MakerNote | No |
| Date, time and time zone of the shot | EXIF | Partly: the camera writes whatever its clock says |
| Exposure settings, camera settings, firmware version | EXIF and MakerNote | No, and they are what makes a sample useful |
| **GPS location** (latitude, longitude, altitude) | GPS block | Yes, turn location data off (below) |
| **Your name and copyright notice** | EXIF `Artist` and `Copyright` | Yes, clear them in the camera menu (below) |
| Image comment, IPTC data | EXIF and XMP | Yes, turn them off in the camera menu |
| **The picture itself**, plus embedded JPEG previews of it | Raw strip and preview images | Choose a scene that shows nothing private |

The serial number deserves a moment's thought. It does not reveal your name by itself,
but it ties every photo you have ever published online, from that body, to the same
camera. Some people use it to trace stolen cameras, which is a reason both to keep it
private and to publish it. Only you can decide.

### Why we cannot "scrub" the file for you

raw.pixls.us only accepts files that are copied straight from the card and **have not
been modified in any way**: an edited file may no longer be what the camera wrote, which
defeats the point of a test sample. Tools that remove metadata change the file, so there
is **no approved scrubbing step** for a public sample. What you can do is stop the camera
from writing optional personal data in the first place (step 1).

If you are not comfortable publishing your body's serial number, that is completely fine.
You can still help a lot: open a [camera support issue](#5-tell-us-about-it) with just the
`ticoraw inspect --json` output, which contains no serial number and no pixels.

### Checking a file yourself

[ExifTool](https://exiftool.org/) can show you everything in a file before you share it:

```sh
exiftool -a -G1 -s your-file.NEF | grep -iE 'serial|gps|artist|copyright|owner|comment'
```

This only reads the file.

## 1. Set up the camera

Menu names below are typical of Nikon Z bodies; yours may differ slightly. Check the
camera's manual if you cannot find one.

- **Image quality:** RAW (RAW + JPEG is fine too; upload only the NEF). Then, under
  **RAW recording** (the name varies between bodies), choose **High efficiency** or
  **High efficiency\***.
- **Location data:** off. This includes location from your phone through SnapBridge, a
  built-in receiver or an add-on GPS unit.
- **Copyright information** (Setup menu, the `Artist` and `Copyright` fields): turn
  "Attach copyright information" off, or clear both fields.
- **Image comment** and **IPTC** presets: off.
- Everything else as you normally shoot. Note the image area (FX, DX, 1:1, 16:9) and the
  tone mode (SDR or HLG): you will report them later.

## 2. Take the picture

raw.pixls.us asks for, and we benefit most from:

- a **well-lit, detailed scene**, for example a daylight landscape, trees, a building
  facade, or a shelf of books: lots of texture and some colour;
- **low ISO**, a lens mounted with the cap off, the image **in focus** and properly
  exposed;
- **landscape orientation** (camera held level).

Please avoid:

- **people**: raw.pixls.us does not accept photographs of people, for legal reasons;
- anything that identifies a private place or person: house numbers, car number plates,
  your home's interior, documents, screens;
- colour charts or test targets, and series of the same scene at many settings. One or
  two good frames are worth more.

## 3. Copy the file without changing it

Copy the NEF from the memory card with your computer's file manager (a card reader, or
the camera mounted as a drive). Do **not** import it with Nikon software, Lightroom, a
phone app or any tool that might rewrite, rename or add metadata to it, and do not edit
it afterwards.

Optional but useful, if you have built `ticoraw`:

```sh
ticoraw --version
ticoraw inspect --json your-file.NEF
```

Keep the output for step 5. If `inspect` says `classic Nikon (not TicoRAW)`, the file is
not High Efficiency: check the image quality setting and shoot again.

## 4. Upload it to raw.pixls.us

[raw.pixls.us](https://raw.pixls.us/) is a public archive of raw samples that darktable,
RawTherapee and other projects use for testing. Uploading there means your file helps
every raw decoder, not just this one.

1. Open <https://raw.pixls.us/> and find the upload form ("Submit a file").
2. Choose your NEF, and tick the declarations: that you own the rights to the file and
   release it into the public domain under **CC0**, and that it was copied straight from
   the card without modification. Only tick them if they are true.
3. Upload. Once the file appears in the archive (it may take a while), copy its link from
   the repository table. A link looks like
   `https://raw.pixls.us/getfile.php/<number>/nice/<file name>.NEF`.

CC0 means anyone may use the file for anything, without credit. That is what lets
`ticoraw` and other projects use it as a test case. If you cannot agree to that, please do
not upload.

## 5. Tell us about it

Open a **camera support** issue:
<https://github.com/lukaj99/ticoraw/issues/new?template=camera_support.yml>

The form asks for the body, firmware version (Setup menu, "Firmware version"), mode,
image area and tone mode, the raw.pixls.us link, and the `ticoraw` output from step 3.
Before pasting `inspect` output, shorten the `path` field if it shows your name.

## What happens next

1. The maintainer downloads the sample from raw.pixls.us and records its SHA-256.
2. It is converted with Adobe DNG Converter, which serves only as a black-box reference,
   and compared with `ticoraw` at every photosite, along with the levels, colour matrix
   and other DNG metadata.
3. If it matches, the body and mode are added to [compat.md](compat.md) and the sample
   joins the conformance set that every future change must pass. If it does not, the
   sample becomes the test case for the fix.
4. The file itself is never stored in this repository: compat.md lists its archive link
   and checksum, and anyone can download it from raw.pixls.us.

Thank you. Every sample makes the project more trustworthy.
