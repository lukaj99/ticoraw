# Convert your first card

This page assumes ticoraw is [installed](install.md): `ticoraw --version` prints a
version number.

## 1. Copy your photos to the computer

Copy the folder from your memory card to your computer first, for example into a folder
called `MyCard` inside your Pictures folder. Use your file manager (Finder, File Explorer,
...) and copy the whole card, including the `DCIM` folder.

You can also convert straight from a card, but then the DNGs are written onto the card.
Working on a copy is simpler and keeps your card as it was.

## 2. Convert

Open a terminal (see the [install guide](install.md) if you need a reminder) and run:

**macOS and Linux**

```sh
ticoraw convert ~/Pictures/MyCard -r
```

**Windows (PowerShell)**

```text
ticoraw convert "$HOME\Pictures\MyCard" -r
```

What this means:

- `convert` is the command that turns NEFs into DNGs;
- `~/Pictures/MyCard` is the folder to convert (`~` is short for your home folder);
- `-r` (short for `--recursive`) also looks inside subfolders. Cameras keep photos in
  subfolders such as `DCIM/100NZ6_3`, so you almost always want it.

> [!TIP]
> You don't have to type the folder's path. Type `ticoraw convert ` (with a space at the
> end), then drag the folder from Finder or File Explorer into the terminal window. Its
> path appears. Add ` -r` and press <kbd>Return</kbd>.

## 3. Read the result

ticoraw prints one line per photo, then a summary. It looks like this:

```text
converted  /home/you/Pictures/MyCard/DCIM/100NZ6_3/DSC_0001.NEF -> /home/you/Pictures/MyCard/DCIM/100NZ6_3/DSC_0001.dng  (13.4 MB -> 28.4 MB, 0.08 s)
skipped    /home/you/Pictures/MyCard/DCIM/100NZ6_3/DSC_0002.NEF: classic NEF (Lossless compressed), not High Efficiency: your editor already supports it

2 files: 1 converted, 1 skipped, 0 failed
read 13.4 MB, wrote 28.4 MB in 0.08 s: 12.70 files/s, 170.5 MB/s read
```

- **converted**: a DNG now sits next to the NEF, with the same name.
- **skipped**: the NEF is not High Efficiency. Your editor already opens it, so ticoraw
  leaves it alone. This is not an error.
- **FAILED**: something went wrong with that file. The line says why, and the other files
  are still converted. See [Troubleshooting](troubleshooting.md).

While it works, a progress line at the bottom shows how far it has got.

Your NEF files are untouched. ticoraw only reads them.

## 4. Open the DNGs in your editor

Import or open the `.dng` files in your editor, just like any raw file. They are raw files,
so you have the same editing freedom as with the NEFs.

A few things worth knowing:

- **Keep your NEFs.** The DNG holds the same raw data, but your NEFs are the camera's
  originals. Keep them as your backup.
- **Colours can look different from the camera's JPEG.** Every raw editor applies its own
  colour and tone "look" to raw files. That is normal and is not caused by the conversion.
- **Shot in HLG tone mode?** Those frames can look about one stop darker in some editors.
  The [FAQ](../faq.md#my-hlg-photos-look-dark) explains why and how to fix it.

## Next time

To put the DNGs in a separate folder instead of next to the NEFs, add `-o` and a folder
name. ticoraw creates the folder and keeps the card's subfolder layout inside it:

```sh
ticoraw convert ~/Pictures/MyCard -r -o ~/Pictures/MyCard-DNG
```

If you run the same command again after adding photos, the photos converted last time
are reported as FAILED with "already exists": ticoraw never replaces a DNG unless you ask.
Add `--skip-existing` to convert only the new ones:

```sh
ticoraw convert ~/Pictures/MyCard -r --skip-existing
```

[Converting photos](../converting.md) explains every option, with examples.
