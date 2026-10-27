# Install ticoraw

Today ticoraw is installed by **building it from its source code** on your own computer.
That sounds harder than it is. You type a few commands, and a tool called `cargo` does the
rest. Plan for about ten minutes the first time.

> [!NOTE]
> Why build it yourself? Ready-made downloads are planned but not published yet, pending a
> patent-licensing review of the compression format inside HE files. Building from source
> on your own computer is how everyone installs ticoraw for now.
> [Patents and downloads](../patents.md) explains more.

You will:

1. open a **terminal**, the window where you type commands;
2. install **Rust**, the programming language ticoraw is written in, with its official
   installer, `rustup`;
3. ask Rust's tool `cargo` to download and build ticoraw.

Rust and ticoraw install into your user account. You can remove both later (see
[Uninstalling](#uninstalling)).

Pick your system:

- [macOS](#macos)
- [Windows](#windows)
- [Linux](#linux)

## macOS

**1. Open Terminal.** Press <kbd>⌘ Command</kbd> + <kbd>Space</kbd>, type `Terminal`, and
press <kbd>Return</kbd>. A window with a text prompt opens. You type each command below
into it and press <kbd>Return</kbd>.

**2. Install Apple's command line tools.** Rust needs them to build programs.

```sh
xcode-select --install
```

A dialog appears. Click **Install** and wait for it to finish. If the terminal says the
tools are already installed, that's fine: go on.

**3. Install Rust.** This is the official command from [rustup.rs](https://rustup.rs):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

When it asks how to proceed, press <kbd>Return</kbd> to choose the default installation.
When it finishes, **quit Terminal and open it again**, so that it finds the new commands.

**4. Check that Rust works.**

```sh
cargo --version
```

It should print a version number of 1.85 or higher, such as `cargo 1.91.0`.

**5. Build and install ticoraw.**

```sh
cargo install --locked ticoraw-cli
```

`cargo` downloads the source code and builds it. This usually takes a minute or two, and it
prints a lot of lines while it works. It ends with a line like `Installed package
ticoraw-cli`.

**6. Check that ticoraw works.**

```sh
ticoraw --version
```

It prints something like `ticoraw 0.1.0`. You're done: go on to [Convert your first
card](first-conversion.md).

> [!TIP]
> **Homebrew users:** the project has a Homebrew formula that also builds ticoraw from
> source on your Mac. It becomes installable with `brew install` once the project's
> Homebrew tap is published, which has not happened yet. Until then, use the steps above.

## Windows

**1. Install Rust.** Go to [rustup.rs](https://rustup.rs) and download `rustup-init.exe`
(the 64-bit version suits almost every PC). Run it.

- If it says you need the **Visual Studio C++ build tools** (MSVC, Microsoft's C++
  compiler and linker), let it install them (choose the option to install Visual Studio
  Community, or install the "Build Tools for Visual Studio" with the **Desktop development
  with C++** workload). Rust uses them to build programs. This download is large and can
  take a while.
- When the installer asks how to proceed, type `1` and press <kbd>Enter</kbd> for the
  default installation.

**2. Open PowerShell.** Click Start, type `PowerShell`, and open **Windows PowerShell**. If
you had a PowerShell window open during the Rust installation, close it and open a new
one, so that it finds the new commands.

**3. Check that Rust works.**

```text
cargo --version
```

It should print a version number of 1.85 or higher.

**4. Build and install ticoraw.**

```text
cargo install --locked ticoraw-cli
```

This usually takes a minute or two, and ends with a line like `Installed package
ticoraw-cli`.

**5. Check that ticoraw works.**

```text
ticoraw --version
```

It prints something like `ticoraw 0.1.0`. Go on to [Convert your first
card](first-conversion.md).

## Linux

**1. Open a terminal** (on most desktops, press <kbd>Ctrl</kbd> + <kbd>Alt</kbd> +
<kbd>T</kbd>, or look for "Terminal" in your applications).

**2. Install a C compiler and curl.** Rust needs a system linker to build programs. Use the
line for your distribution:

```sh
sudo apt install build-essential curl          # Debian, Ubuntu, Linux Mint
sudo dnf install gcc curl                      # Fedora
sudo pacman -S --needed base-devel curl        # Arch, Manjaro
```

**3. Install Rust** with the official command from [rustup.rs](https://rustup.rs):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Press <kbd>Enter</kbd> for the default installation, then close the terminal and open a
new one. (Your distribution's own `rust` or `cargo` package works too, if it is version
1.85 or newer. Debian's and Ubuntu's packages are usually older; an older one fails with
``feature `edition2024` is required``, see
[troubleshooting](troubleshooting.md#feature-edition2024-is-required-or-requires-rustc-185-or-newer).)

**4. Check, build and install.**

```sh
cargo --version          # 1.85 or higher
cargo install --locked ticoraw-cli
ticoraw --version
```

Go on to [Convert your first card](first-conversion.md).

## From a copy of the source code

If you prefer to look at the code before you build it, clone the repository and install
from the clone. This needs [git](https://git-scm.com/) as well as Rust.

```sh
git clone https://github.com/lukaj99/ticoraw
cd ticoraw
cargo install --path crates/ticoraw-cli
```

Both ways install the same program, called `ticoraw`, into Rust's `bin` folder
(`~/.cargo/bin` on macOS and Linux, `%USERPROFILE%\.cargo\bin` on Windows).

## Updating

Run the install command again:

```sh
cargo install --locked ticoraw-cli
```

`cargo` installs the newest release from crates.io when there is one. To rebuild it anyway, add `--force`.
To update Rust itself, run `rustup update`.

## Uninstalling

```sh
cargo uninstall ticoraw-cli     # removes ticoraw
rustup self uninstall           # removes Rust too, if you no longer need it
```

Your photos and the DNGs you made are not touched by either command.

## If something went wrong

The [troubleshooting page](troubleshooting.md#installing) covers the usual install
problems, such as `command not found` or a missing linker.
