# Troubleshooting

Find the message you see, or the problem you have, below. If none of them fits, see
[Getting help](../help.md).

## Installing

### `cargo: command not found` or `ticoraw: command not found`

The terminal doesn't know where Rust put its programs yet.

- Close the terminal window and open a new one. A terminal only learns about new programs
  when it starts.
- On macOS and Linux you can also run `. "$HOME/.cargo/env"` in the open window.
- Check that Rust's `bin` folder exists: `~/.cargo/bin` on macOS and Linux,
  `%USERPROFILE%\.cargo\bin` on Windows. If it does not, run the Rust installer again.

### `xcrun: error: invalid active developer path` (macOS)

Apple's command line tools are missing. Run `xcode-select --install`, click **Install**,
then run the `cargo install` command again.

### ``linker `cc` not found`` (Linux)

Rust needs a C compiler to link programs. Install it with your package manager
(`sudo apt install build-essential` on Debian and Ubuntu, `sudo dnf install gcc` on
Fedora, `sudo pacman -S --needed base-devel` on Arch), then try again.

### ``linker `link.exe` not found`` (Windows)

Rust needs Microsoft's C++ build tools. Install "Build Tools for Visual Studio" with the
**Desktop development with C++** workload (the Rust installer offers to do this), open a
new PowerShell window and try again.

### "requires rustc 1.85 or newer"

Your Rust is too old. Run `rustup update`, then try again. If you installed Rust from your
Linux distribution's packages, install it with [rustup](https://rustup.rs) instead, or
upgrade the package.

## Converting

### `0 files: 0 converted, 0 skipped, 0 failed`

ticoraw found no NEF files.

- Did you add `-r`? Cameras keep photos in subfolders (`DCIM/100NZ6_3`, ...), and without
  `-r` ticoraw only looks in the folder you named.
- Is the path right? Drag the folder into the terminal window to paste its exact path.

### `skipped ... classic NEF ..., not High Efficiency: your editor already supports it`

Nothing is wrong. That photo uses Nikon's older raw format, which your editor opens as it
is. ticoraw only converts HE and HE\* files.

### `FAILED ... already exists (use --force to replace it, or --skip-existing)`

A DNG with that name is already there, usually from an earlier conversion. ticoraw never
replaces a file unless you ask.

- Add `--skip-existing` to leave existing DNGs alone and convert the rest.
- Add `--force` to replace them.

### `FAILED ...: reading: No such file or directory`

The path you typed does not exist. Check the spelling, or drag the folder into the terminal
window instead of typing it. On Windows, put paths with spaces in quotes:
`ticoraw convert "$HOME\Pictures\My Card" -r`.

### `warning ... no colour profile for ...`

The photo comes from a camera ticoraw does not have colour data for yet. The DNG is still
written and its raw data is correct, but its colours will be wrong in raw editors. Please
[tell us about the camera](https://github.com/lukaj99/ticoraw/issues/new?template=camera_support.yml)
so it can be added.

### `FAILED ... this High Efficiency variant is not supported yet`

Your camera wrote a kind of HE file that ticoraw has not seen before. That is exactly the
file we need to improve ticoraw. Please run:

```sh
ticoraw report /path/to/that/photo.NEF
```

and paste its output into a [bug report](https://github.com/lukaj99/ticoraw/issues/new?template=bug_report.yml).
The report holds technical facts about the file only: no pixels, serial numbers, GPS
position, names, dates or file paths. Please do not attach the NEF itself.

### `FAILED ... truncated ...` or `container: ...`

The file is damaged, incomplete (for example, a copy that was interrupted) or not a Nikon
raw file. Copy it from the card again and retry. If a file that opens fine in Nikon's own
software fails, please send a `ticoraw report` as above.

## After converting

### My editor still won't open the DNG

Make sure you opened the `.dng` file, not the `.NEF`. If the DNG fails to open, please
[report it](https://github.com/lukaj99/ticoraw/issues/new?template=bug_report.yml) with the
name and version of your editor and the output of `ticoraw report` for the NEF.

### Colours or brightness differ from the camera's JPEG or from Lightroom

Each raw editor renders raw files with its own colour profile and tone curve, so their
results differ. The raw values in the DNG are identical to Adobe's. Frames shot in **HLG
tone mode** have an extra twist: see
[the FAQ](../faq.md#my-hlg-photos-look-dark).

### The DNGs are bigger than the NEFs

That is expected: see [Why are DNGs bigger than my NEFs?](../faq.md#why-are-dngs-bigger-than-my-nefs)
