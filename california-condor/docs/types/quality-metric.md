# Quality Metric

`METRIC` selects the perceptual quality metric for Target Quality and Quality Check. Used by `--target-metric`, `--metric` (see [init]\(../commands/init.md\), [target-quality]\(../commands/target-quality.md\), [quality-check]\(../commands/quality-check.md\), [pipeline](../commands/condor.md)).

| Value           | Metric                                                        | Plugin Requirement                                  |
| --------------- | ------------------------------------------------------------- | --------------------------------------------------- |
| `ssimulacra2`   | Structural SIMilarity Unveiling Local And Compression Artifacts | Vship (GPU, recommended) or vszip (CPU)            |
| `butteraugli`   | butteraugli Infinite-Norm                                     | Vship (GPU, recommended) or julek plugin (CPU)      |
| `butteraugli-3` | butteraugli 3-Norm                                            | Vship (GPU, recommended) or julek plugin (CPU)      |
| `xpsnr`         | Extended Perceptually Weighted PSNR (min of Y, U, V)          | vszip (CPU)                                         |
| `cvvdp`         | ColorVideoVDP                                                 | Vship (GPU required)                                |
| `vmaf`          | Video Multi-Method Assessment Fusion                          | Unimplemented                                       |

## Native Vship

When the Vship library is installed, Condor computes `ssimulacra2`, `butteraugli`, `butteraugli-3` and `cvvdp` with it directly instead of through the Vship VapourSynth plugin, and uses the plugins above otherwise. Set the `CONDOR_VSHIP` environment variable to the path of the library to choose one, or to `none` to always use the plugins. See [Installing Vship](../../../condor-vship/README.md#installing-vship).

## Default

If not specified, `ssimulacra2` is used.

## Examples

- `> condor target-quality --metric ssimulacra2 --target 85`
- `> condor quality-check --metric xpsnr --profile fast`
