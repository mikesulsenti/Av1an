# Condor Vship

Native [Vship][vship] GPU metrics for Condor: [SSIMULACRA 2][ssimulacra2], [butteraugli][butteraugli] and [ColorVideoVDP][cvvdp].

Condor Vship calls the Vship library directly instead of going through the Vship VapourSynth plugin. It scores frames from [av-decoders][av-decoders], the same decoders Andean Condor uses, or from VapourSynth frames and nodes with the `vapoursynth` feature. Andean Condor uses it for Target Quality and Quality Check whenever Vship is installed and falls back to the VapourSynth plugins otherwise.

## Installing Vship

Vship is **loaded at runtime and never linked** into Condor. It is a separate download that you can update on your own without a new Condor build. Vship ships one library for both its API and its VapourSynth plugin, so an installed Vship plugin works too.

Condor looks for the library in this order:

1. The path in the `CONDOR_VSHIP` environment variable. Set it to `none` to disable native Vship and use the VapourSynth plugins instead.
2. Next to the Condor executable: `vship.dll` or `libvship.dll` on Windows, `libvship.so` on Linux.
3. The system library search path: `PATH` on Windows, `LD_LIBRARY_PATH` and the system library directories on Linux.
4. The Vship VapourSynth plugin, if VapourSynth has loaded it.

The first library that loads, is Vship 5 or newer, and passes Vship's GPU check is used. Condor logs which library it uses.

### Windows

The Windows releases of Condor include `vship.dll`, the Vulkan build of Vship, next to `condor.exe`. Vulkan works on AMD, Intel and NVIDIA GPUs with current drivers.

To use another build, download it from the [Vship releases][vship-releases] and replace `vship.dll` (or set `CONDOR_VSHIP` to its path):

| Release asset         | GPUs                       |
| --------------------- | -------------------------- |
| `libvship_VULKAN.dll` | AMD, Intel and NVIDIA      |
| `libvship_NVIDIA.zip` | NVIDIA (CUDA)              |
| `libvship_AMD.zip`    | AMD (HIP)                  |

### Linux

Build Vship from source with the backend for your GPU. The Vulkan backend needs the Vulkan headers and loader, for example `pacman -S vulkan-headers vulkan-icd-loader` on Arch Linux. The CUDA and HIP backends need `nvcc` or `hipcc` instead. See the [Vship installation guide][vship-install] for details.

```bash
git clone --branch v5.1.1 https://codeberg.org/Line-fr/Vship.git && cd Vship
make build BACKEND=Vulkan   # or BACKEND=Cuda, BACKEND=HIP
sudo make install PREFIX=/usr
```

`make install` installs `libvship.so` into the system library directory, where Condor finds it, and links it as a VapourSynth plugin. If you install to another prefix, such as the default `/usr/local`, make sure the library directory is in `LD_LIBRARY_PATH` or the `ld.so` configuration, or point `CONDOR_VSHIP` at the library.

The [Docker image](../README.md#installation) includes the Vulkan build of Vship. Containers need GPU access, such as `--device /dev/dri` or the NVIDIA Container Toolkit.

### Checking the installation

After running Target Quality or Quality Check, look for `Using native Vship` in the Condor log file (`--logs`, `./logs/condor.log` by default). If Vship cannot be loaded, the log explains why after `Native Vship unavailable` and Condor uses the VapourSynth plugins.

## Using the library

```rust
use condor_vship::{Metric, Settings, Vship};

let vship = Vship::find(&[])?;
let mut reference = av_decoders::Decoder::from_file("reference.y4m")?;
let mut distorted = av_decoders::Decoder::from_file("distorted.y4m")?;

let settings = Settings::new(Metric::SSIMULACRA2);
let scores = vship.compare_decoders(&settings, &mut reference, &mut distorted, |index, score| {
    println!("frame {index}: {score}");
    Ok::<_, std::convert::Infallible>(())
})?;
```

- `Vship::find` and `Vship::global` load the library, `Vship::load` loads a specific path.
- `Vship::score_frames` scores any iterator of frame pairs with `Settings::streams` scorers in parallel, holding at most two pairs per scorer in memory.
- `Vship::compare_decoders` decodes two `av_decoders::Decoder`s and scores them.
- `Vship::score_nodes` (`vapoursynth` feature) scores two VapourSynth nodes, with each scorer requesting its own frames so VapourSynth filters run in parallel.
- `Scorer` scores one pair of frames at a time, for custom pipelines.

Scores are reported in frame order. CVVDP is a video metric: unless `disable_temporal` is set, each score covers every frame before it, and a single scorer processes the frames in order.

### Color

Frames are described to Vship with their format and a `ColorDescription` using ITU-T H.273 values. VapourSynth frames provide their `_Matrix`, `_Transfer`, `_Primaries`, `_ColorRange` and `_ChromaLocation` properties. Values the frames do not carry come from `Settings::reference_color` and `Settings::distorted_color`, then from the defaults of the Vship VapourSynth plugin: BT.709 (BT.601 below 650 lines), limited range YUV and full range RGB.

Unlike the Vship VapourSynth plugin, which always converts with those defaults, frame properties are honored. Scores of HDR or otherwise tagged clips can therefore differ from the plugin, and are more accurate.

## Testing

```bash
cargo test -p condor-vship --all-features
```

Tests that need Vship skip themselves when it is not installed. Set `CONDOR_VSHIP_REQUIRED=1` to make them fail instead, as CI does. Without a GPU, the Vulkan build of Vship runs on the CPU with Mesa's lavapipe driver (`vulkan-swrast` on Arch Linux), which is slow but enough for the tests. The `vapoursynth` tests compare native scores with the Vship VapourSynth plugin, loading it from `CONDOR_VSHIP` when VapourSynth does not autoload it.

## Benchmarks

```bash
cargo bench -p condor-vship
```

The benchmarks score synthetic 720p frames with each metric, one frame at a time and with 1, 2 and 4 parallel scorers. They need Vship and a GPU; without Vship they are skipped.

## License

Condor Vship is part of Condor and licensed under the GPL-3.0.

The Vship API declarations in [`src/ffi.rs`](src/ffi.rs) are derived from `VshipAPI.h` and `VshipColor.h`, which their author has made available under the MIT license; the notice is included in that file. The Vship library itself is distributed under its own [MIT NON-AI license][vship-license] and is not part of this crate. Builds that redistribute it, like the Windows releases, include that license next to it.

[vship]: https://codeberg.org/Line-fr/Vship "Vship: Fast Metric Computation on GPU"
[vship-releases]: https://codeberg.org/Line-fr/Vship/releases "Vship releases"
[vship-install]: https://codeberg.org/Line-fr/Vship#installation "Vship installation"
[vship-license]: https://codeberg.org/Line-fr/Vship/src/branch/main/LICENSE "Vship license"
[av-decoders]: https://github.com/rust-av/av-decoders "av-decoders"
[ssimulacra2]: https://github.com/cloudinary/ssimulacra2 "SSIMULACRA 2"
[butteraugli]: https://github.com/google/butteraugli "butteraugli"
[cvvdp]: https://github.com/gfxdisp/ColorVideoVDP "ColorVideoVDP"
