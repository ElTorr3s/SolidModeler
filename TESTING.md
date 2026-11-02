# Trying SolidModeller on Omarchy

Everything built so far is on `main`: the truck kernel with booleans, the
part history with versions, branches and merges, the sketch solver,
topological naming, the desktop app, CI with fuzzing, and the OpenCASCADE
backend with sandboxed STEP import.

## 1. Install

Omarchy already ships the Vulkan driver and loader the app needs. You need
Rust and a C toolchain:

```sh
sudo pacman -S --needed rustup base-devel git vulkan-icd-loader
rustup default stable          # Rust 1.95 or newer
```

If `vulkaninfo --summary` (from `vulkan-tools`) lists no GPU, install your
driver: `vulkan-radeon`, `vulkan-intel` or `nvidia-utils`.

## 2. Build

```sh
git clone https://github.com/ElTorr3s/SolidModeler.git
cd SolidModeler
cargo build --release          # about 5 to 10 minutes the first time
```

Binaries land in `target/release/`: `solidmodeller` (the app) and `smcli`
(the command line).

## 3. Open the app

```sh
./target/release/smcli demo ~/block.smp     # a block with a pocket and a notch
./target/release/solidmodeller ~/block.smp
```

It opens as a native Wayland window with the app id `solidmodeller`, so
Hyprland window rules can match it, and it takes its colours from your
current Omarchy theme.

Open the app with a file. Started with no file it shows the same demo part,
but that part cannot be saved, and the File menu has no Open yet: drag a
`.smp` file onto the window to open another one.

## 4. Things to try

In the app:

1. **Navigate like Inventor:** middle drag pans, Shift+middle drag (or hold
   F4 and left drag) orbits, the wheel zooms at the cursor, F6 is the home
   view and Home is zoom all. Click a face of the view cube to look at it.
2. **Switch the Omarchy theme** (Omarchy menu → Style → Theme) while the
   app is open. It should restyle within a second, light themes included.
3. **Pick faces:** click a face; it highlights. Esc clears it.
4. **Roll back the history:** right-click a feature in the model browser
   and choose "Move EOP below this". Right-click "End of Part" and choose
   "Move EOP to End" to roll forward again.
5. **Suppress a feature:** right-click Extrusion2 → "Suppress Features" and
   watch the pocket disappear.
6. **Save:** Ctrl+S. Closing with unsaved changes asks Save / Don't Save /
   Cancel.

On the command line, branch the part's history and merge it back (close
the app first):

```sh
S=./target/release/smcli
$S commit ~/block.smp "Initial"
$S branch ~/block.smp deeper
$S checkout ~/block.smp deeper
$S set-distance ~/block.smp Extrusion2 -8    # pocket 8 mm deep instead of 5
$S commit ~/block.smp "Deeper pocket"
$S checkout ~/block.smp main
$S set-distance ~/block.smp Extrusion2 -3    # and 3 mm on main
$S commit ~/block.smp "Shallow pocket"
$S merge ~/block.smp deeper            # refused: both sides changed the pocket
$S merge ~/block.smp deeper theirs     # settle it with the branch's 8 mm
$S log ~/block.smp
$S rebuild ~/block.smp ~/block.step    # export STEP and open it elsewhere
```

Then open `~/block.smp` in the app again: the pocket is 8 mm deep, and
saving from the app keeps the version history (`smcli log` still shows it).

## 5. What is not there yet

- **No modelling in the app yet.** You cannot draw sketches or add features
  in the window; parts come from `smcli demo` and `smcli set-distance`. The
  sketch solver and topological naming are libraries with tests but are not
  wired into the app.
- **History is command line only.** The app keeps and saves versions but has
  no version or branch panel.
- **No File → Open**, and a part started without a file cannot be saved.
- **No STEP import in the app yet.** It lives in the OpenCASCADE crate,
  which the app and `smcli` do not use yet. STEP export works.
- The app still runs on the `truck` kernel; the OpenCASCADE backend is built
  and tested on its own.
- `smcli log | head` panics when `head` closes the pipe early.

## Optional: the OpenCASCADE crate

`sm-occt` is left out of a plain `cargo build` because it needs
OpenCASCADE:

```sh
sudo pacman -S opencascade
cargo test -p sm-occt
```

It is tested against OpenCASCADE 7.6 (Ubuntu), 7.9.3 and 8.0.1, and CI runs
it on Arch's own package.

## If something goes wrong

- Start the app from a terminal and keep what it prints.
- `cargo test` runs the whole suite and should pass.
