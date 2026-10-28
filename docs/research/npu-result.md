# NPU: the entry LG ships, what it accepts, and what LumaTV runs

## Resume here

**Result.** Camera detection runs on the CPU: EfficientDet-Lite0 int8 through XNNPACK, in a 32-bit worker. Nothing else on the set takes our own network today:

- **NPU.** LG's NPU is reached through the TFLite external-delegate plugin `tflite_plugin_create_delegate` in `/usr/lib/libtflite-npu-delegate.so.1`. The plugin takes
  only a graph that contains LG's own custom op `lgnpu_custom_op`, whose `custom_options` are an NPU package LG compiled offline, and refuses every graph of builtin ops,
  including LG's `FitTV_Detector_Yolov3_QAT_SoC.tflite` (the reason `PLAN.md` saw it "apply to none of LG's shipped models"). No compiler or package format is
  published. The NNAPI route is no route either: LG's "online" NN HAL (`libwebos_nnhal.so`, `lgnpu_online_nnhal_test`) is a stand-in whose execute call sleeps 1 ms and
  reports it as the time on the device (section "The online NN HAL"). Both stay wired in the probe, with a check that catches a device that does not compute.
- **LG's own AI programs** (section "LG's own AI packages") are not ours to call. `SGNET_o22` (scene, genre, face measure) and `GNMNET_o22` (grain, noise) are programs for
  the display engine's neural unit that LG's picture service loads and feeds with the live picture: not `.lgnpu` files, so `run_lgnpu` refuses them, and no luna method
  returns their output. What LG does expose is the programme's **genre**, computed on the CPU by its `DNN_GNET` and served by `nn-proxy`: the usable input for light sync.
- **GPU** (section "The GPU route"): TFLite's GPU delegate on OpenCL is built into the probe (`--gpu`, `--gpu-info`, `--gpu-coverage`). It takes 266 of Lite0's 267 nodes and 101 of
  SSD-MobileNetV2's 102 (the post-processing op stays on the CPU). On the Mali-G52 it is estimated at 30 to 140 ms a frame, no faster than the CPU at Home, on a GPU the UI
  already keeps 10 to 70 % busy, plus seconds of kernel build at start. An optional backend, off; TV step 6d measures it.

What runs where:

| Where | What runs there | Speed on the TV | Camera detection |
|---|---|---|---|
| NPU `/dev/npu0` (DNE, 0.4 TOPS) through `libnpu` `.lgnpu` packages | LG's AI picture (`ai_pq`, 0.29 TOPS); LG's person-detector packages `FitTV_Detector_Yolov3_V1/V2` (named, not on the set) | no package of ours; if one existed 5 ms at full rate, 16 to 20 ms at 25 to 30 % [INFERENCE: arithmetic] | no: no compiler |
| Display engine's neural unit (`/dev/lne_pqe_ipc`) through `libaibspLib` | `SGNET_o22` scene, genre, face measure; `GNMNET_o22` grain, noise: on the live picture | LG's, per frame in the display path | no: not callable; the genre it feeds is read from nn-proxy for light sync |
| CPU, 3 to 4 A76 at 1.4 GHz, TFLite with XNNPACK | **EfficientDet-Lite0 int8** (SSD-MobileNetV2 int8 as the fallback) | Lite0 60 to 120 ms, SSD 35 to 60 ms [INFERENCE; step 4 measures] | **chosen** |
| GPU, Mali-G52 (3 cores), TFLite GPU delegate on OpenCL | the same models in FP16 (int8 files run in float on the GPU) | Lite0 30 to 140 ms, SSD 22 to 110 ms, 2 to 10 s kernel build cold [INFERENCE; step 6d measures] | optional second backend, off |
| CPU, LG's `BODY` detector (`libai-inference-interface`) | `FitTV_Detector_Yolov3_QAT_SoC`, people only | 421 ms (4 threads, builtin kernels, `PLAN.md`) | no: slower, no animals |

**Done offline (2026-10-28), nothing run on the TV.**

- Symbols, strings and JSON of the 24 ELF files and 7 JSON files copied off the set (sizes and hashes in the copy's `inventory.txt`,
  scratch under `/tmp/lumatv-npu-tv`). No disassembly.
- LG's delegate run as a black box on the host (`tests/lg/run.sh`, 18 checks): its options, the graphs it takes and refuses, the calls it makes
  into `libnnrt`, and the TFLite structure layout it was built against (2.13). LG's online NN HAL test run the same way (`tests/lg/hal_mock.sh`,
  19 checks, system calls traced under qemu-arm-static): what the HAL does and does not touch.
- `npuprobe/`: `lumatv-npu-probe` (TFLite 2.13.1 static, 32-bit ARM soft-float and x86-64), a stub delegate with LG's plugin surface,
  a stub NNAPI runtime for its NNAPI path, a stub GPU delegate and a stub OpenCL loader for the GPU route, 100 tests that pass on the host build and 99 on
  the ARM one (under qemu), and the TV slot script `tv-probe.sh` with `gpu-ui-impact.py`.
- The GPU delegate itself on the host: `tests/pocl/run.sh` runs the probe's `--gpu` against PoCL (a CPU OpenCL implementation, Debian 12's packages unpacked under
  `/tmp`) with a shim that gives its device the GPU type. Lite0 int8 through TFLite's delegate takes 266 nodes in one kernel and finds the same seven people in
  `5people.jpg` as the CPU (`detections vs CPU: the same`); the float32 Lite0 and SSD-MobileNetV2 take every node in one kernel and match XNNPACK to 8e-6; the kernel
  cache brings Lite0's set-up from 5.5 s to 0.3 s. The times are a CPU's and say nothing about the Mali.
- The models, `npuprobe/fetch-model.sh` (`--gpu`: the two float32 networks).

**Open (needs a TV slot, below).** The device's own TFLite version (the delegate is only safe in a process with the same 2.13 layout),
whether any `.tflite` on the set carries an NPU package, the speed of the model on the A76s, whether the set has `libneuralnetworks.so`
(the NNAPI runtime; it would change nothing about speed while the HAL behind it is the stand-in); whether OpenCL has a driver on the set at all, the GPU's real speed and
what it does to the UI's page flips (steps 6c and 6d); what nn-proxy answers for the genre, and which process runs LG's AI libraries (step 6e).

## Evidence key

`SYM` exported symbols, strings and ELF headers of the copied files (read with a symbol and string reader; no disassembly).
`JSON` the luna permission, role and model-list files. `RUN` behaviour of LG's `libtflite-npu-delegate.so.1.0.0` loaded as a black
box with fake `libnnrt` and `libPmLogLib` (`tests/lg/run.sh`), and of LG's own `lgnpu_online_nnhal_test` with its library, traced (`tests/lg/hal_mock.sh`). `SRC` public sources (TensorFlow, webosose). `TV` earlier measurements
recorded in `hardware-census.md` and `PLAN.md`. `[INFERENCE]` not observed; the step that settles it is named.

## Architecture

`SYM` All 24 ELF files are 32-bit ARM, EABI5, `e_flags 0x5000200` (soft-float calling convention, VFPv3 and NEON code, Thumb-2),
interpreter `/lib/ld-linux.so.3`, at most GLIBC_2.34 and GLIBCXX_3.4.29 (GCC 11, glibc 2.35 era). Built as `lib32-*` Yocto recipes:
`lib32-tflite-npu-delegate/1.0.0-18-r3`, `lib32-offline-nnhal/2.0.1-1240.ptl4tv.9-r6`, `lib32-libnpu/2.0.1-1240.ptl4tv.9-r9`,
`lib32-edgeai-vision/1.0.0-93.webos4tv.3-r10`, `lib32-tflite-auto-delegation/1.0.0-46-r2`. The kernel is 64-bit, the userland 32-bit
(`hardware-census.md`), so anything that loads these libraries is a 32-bit soft-float ARM process; the 64-bit helper cannot. The probe is
built for that ABI (GCC 11, glibc 2.35 sysroot) and a later detector service follows the `lumatv-jpeg` pattern: a 32-bit process the
helper starts with a socket.

## The entry: `tflite_plugin_create_delegate`

`SYM` `libtflite-npu-delegate.so.1` (17,804 bytes, sha256 `cff29002…c466fcf`) exports:

| Export | Use |
|---|---|
| `tflite_plugin_create_delegate(char** keys, char** values, size_t n, void (*report_error)(const char*))`, `tflite_plugin_destroy_delegate(TfLiteDelegate*)` | the TFLite external-delegate plugin ABI (`tensorflow/lite/delegates/external`); what `benchmark_model --external_delegate_path` calls. **Chosen entry.** |
| `webos::npu::tflite::TfLiteNpuDelegateCreate(const NpuDelegateOptions&)`, `TfLiteNpuDelegateDelete`, `NpuDelegateOptions(keys, values, n, report_error)` | the C++ form of the same thing; LG's `libedgeai-vision` links it directly (`aif::NpuDelegate`). |
| `webos::npu::tflite::PrepareImpl`, `CustomOpInit/Prepare/Invoke/Free`, `CustomOpKernel::*` | the delegate's `Prepare` and the kernel it installs. |

It has no `DT_NEEDED` on a TFLite library: `TfLiteDelegateCreate`, `TfLiteIntArrayCreate`, `TfLiteIntArrayFree` and
`tflite::Subgraph::SetCustomAllocationForTensor(int, const TfLiteCustomAllocation&, long long)` are left undefined and bind to whatever
the process exports (on the set, `libtensorflow-lite.so`; in the probe, its own static TFLite, linked `-rdynamic`). It needs
`libnnrt.so.1` and `libPmLogLib.so.3`. `RUN` With `RTLD_NOW` it loads in the probe once those four symbols are exported.

Options (`SYM` strings `use_custom_alloc`, `true`, `min_freq`, `stoi`, `Wrong key entered in NpuDelegateOptions `; `RUN` behaviour):

| Key | Effect |
|---|---|
| `min_freq=N` | parsed with `std::stoi`; handed to `Nnrt::PrepareOpGraph(graph, priority)` as `priority` (N=30 gave `priority=30`, none gave 0). The name says frequency; the NNRT strings say "minFreq is wrong %d" and "prepare with priority %d". A non-number throws `std::invalid_argument("stoi")` out of the create call: uncaught it aborts the host process, so the probe catches it. |
| `use_custom_alloc=true` | zero-copy tensors. `prepare` (not the first `invoke`) registers the input and output with `AddInputTensor`/`AddOutputTensor(..., custom_alloc=1)`, takes the buffer pointer and size from the `nnrt::Tensor` that `getInputTensor`/`getOutputTensor` return, hands them to TFLite with `Subgraph::SetCustomAllocationForTensor(tensor, {ptr, bytes}, flags=1)` (skip the alignment check), and calls `PrepareOpGraph`; `invoke` is then only `ExecuteOpGraph`. Any other value is false: the tensors are registered at the first `invoke` with TFLite's own buffers. |
| anything else | `report_error("Wrong key entered in NpuDelegateOptions <key>")`, and the delegate is created anyway. |

What `Prepare` accepts (`RUN`, `tests/lg/lg_contract.cc`, `SYM` strings `lgnpu_custom_op`, `Nodes doesn't have customOp node for NPU`):

| Graph handed to `delegate->Prepare` | Result |
|---|---|
| only builtin ops (conv2d, add), or a custom op with no name | returns 1; PmLog level 3 `NPUDELEGATE`: `%s failed : Nodes doesn't have customOp node for NPU` (`PrepareImpl`). No call into `libnnrt`. (With a null PmLog context the delegate crashes inside `PrepareImpl`: the context must be real, as on the set.) |
| one node with `builtin_code == CUSTOM` and `custom_name == "lgnpu_custom_op"`, alone or among builtin ops, in any position | `ReplaceNodeSubsetsWithDelegateKernels` with that one node; the registration it passes is named `lgnpu_custom_op`, version 1. |

`flags` is `kTfLiteDelegateFlagsAllowDynamicTensors` (1), so TFLite does not prepare the graph first and a graph whose custom op has no CPU
kernel can be handed to it unresolved (`SRC` `Subgraph::ModifyGraphWithDelegate`). The probe registers a placeholder op anyway, so that
delegates without the flag work too.

The kernel (`RUN`, libnnrt faked, calls logged): `init` -> `Nnrt::getInstance().LoadOpGraph(custom_options, size)`, with the node's
`custom_options` bytes as the package (8 bytes in, 8 bytes seen); without `use_custom_alloc`, the first `invoke` -> `AddInputTensor(graph,
type, bytes, tensor data, flags 4, dims, custom_alloc=0)`, `getInputTensor`, `AddOutputTensor`, `getOutputTensor`, `PrepareOpGraph(graph,
priority)`; every `invoke` -> `ExecuteOpGraph(graph)`; `free` -> `UnLoadOpGraph(graph)`. The type code for float32 is 6.

Below it (`SYM`): `libnnrt` (`webos::nnrt::Nnrt`: `LoadOpGraph`, `PrepareOpGraph(id, priority)`, `ExecuteOpGraph`, tensors) over
`libwebos_offline_nnhal` (`webos::hal::nn::lgnpu_nn_hal`: `init(package)`, `prepare`, `invoke`, `alloc_tensor_memory`) over `libnpu`
(`lgnpu_init/prepare/invoke/free`, `lgnpu_set_affinity/period`), which parses the package with a flatbuffer `PackageParser` (program,
parameters, input and output tensors, feature-buffer size, private data for the DNE or for "LNX", the CPU) and talks to `/dev/npu0` by ioctl
(`hardware-census.md`). `libnpu` converts tensor I/O only between UINT8 and FLOAT32 ("Currently LGNPU supports quantization/dequantization
only between UINT8 and FLOAT32"), so an NPU model is a single `lgnpu_custom_op` node between a UINT8 or float input and output; the
network, its int8 weights and their scales live in the package.

### TFLite version

`RUN` The `TfLiteRegistration` the delegate passes by value is **ten words**: `init, free, prepare, invoke, profiling_string, builtin_code
(32), custom_name, version (1), registration_external (0), async_kernel (0)`. Only a host that expects ten words reads the next argument
(`nodes_to_replace`, one entry) correctly; a host built for 8, 9 or 11 words gets null or garbage and crashes inside
`Subgraph::ReplaceNodeSubsetsWithDelegateKernels`. `SRC` `TfLiteRegistration` has 8 fields in 2.9 (and earlier), 9 in 2.10 to 2.12, 10
in 2.13, 11 from 2.14; `TfLiteContext`'s first 23 fields are the same from 2.9 to 2.16. The delegate was therefore built against
TensorFlow Lite **2.13.x**. `SYM` `libedgeai-vision` also needs the 3-argument `InterpreterBuilder` (2.9 or later).

webosose's open recipe ships 2.9.3 (`SRC` `meta-webosose/.../tensorflow-lite_2.9.3.inc`); the set's `libtensorflow-lite.so` is not among the
copied files, so its version is read at TV step 1 (`lumatv-npu-probe --system-info`; or `strings /usr/lib/libtensorflow-lite.so | grep -E
'^2\.[0-9]+\.[0-9]+$'`). The probe is built with 2.13.1, so LG's delegate is safe in it only if step 1 reports 2.13. LG's models were
written by TF 2.9.1 (`CONVERSION_METADATA` of the QAT detector); that says nothing about the runtime.

## What runs on the NPU today, and what does not

`SYM`, `JSON`

- **LG's NPU user** is the AI picture path (`ai_pq`, 0.29 of the NPU's 0.4 TOPS in `/usr/share/aif/model_info/model_npu_tops.json`; the only
  registered user in `PLAN.md`). `/usr/share/aibsp/{GNMNET,SGNET}_o22_{text,param}.dat` look like its programs and parameters [INFERENCE: by name;
  the loader is not among the copied files]. `libaibsp` runs float TFLite models (`DNN_GNET`, `DNN_SQM1-3`, `sound_DNN` in `/usr/share/aiservice`,
  listed by `default_models.json`) and skips initialisation when `/var/aibsp/skip` exists. `TV` `npu_utilization` reads 0 and
  `npu_process_list` is empty at Home; `aipq compatibility : false`.
- **LG's EdgeAI vision stack** (`libedgeai-vision`, extensions `libedgeai-vision-base` and `-object-detection`, allowed by
  `/usr/share/aif/edgeai.config`): TFLite with a delegate chosen per detector (`mainDelegate`: `npu_delegate`, `nnapi_delegate`,
  `gpu_delegate`; XNNPACK for the CPU; an `AutoDelegateSelector` with policies `CPU_ONLY` and `LOAD_BALANCING`). Its person detectors are
  `person_yolov3_v1_npu`, `person_yolov3_v2_npu` (models `FitTV_Detector_Yolov3_V1/V2.tflite` [INFERENCE: paired by name], not on the set) and `person_yolov3_qat`,
  `_ptq`, `_ptq_rtk` (`FitTV_Detector_Yolov3_{QAT,PTQ}_SoC.tflite`, `_PTQ_RTK`). Only the QAT SoC file is on the set.
- **That file** (4.39 MB, written by TF 2.9.1, "MLIR Converted") has 111 builtin ops: QUANTIZE 1, PAD 23, CONV_2D 52, DEPTHWISE_CONV_2D 22, ADD 9,
  RESIZE_NEAREST_NEIGHBOR 2, CONCATENATION 2; uint8 input [1,270,480,3] (scale 1/255) then int8 activations with per-channel int8 weights; six int8
  raw heads (9x15, 18x30, 36x60), no post-processing; 1,264 MMAC. No custom op: it is the quantisation-aware-trained input of LG's compiler, not
  its output. `TV` On the CPU it takes 421 ms with 4 threads (`PLAN.md`); the delegate applied to none of LG's models there, which is the
  refusal above.
- **No NPU package is known on the set.** `auto_delegation_test` names `/model/FitTV_Pose2D.tflite` for its `webosnpu` case; it is not shipped.
  TV step 2 searches every `.tflite` for `lgnpu_custom_op`.
- **`libwebos_nnhal.so`** (21,900 bytes) is an NN HAL 1.2 shaped device, `lgnpu`, `v1.0`, and a stand-in: see the next section.

## The online NN HAL

Asked: LG ships `libwebos_nnhal.so`, `lgnpu_online_nnhal_test`, `lgnpu_nn_hal_test`, `run_lgnpu` and `nn-proxy`; does one of them compile a
standard model for the NPU at run time? No. `SYM`, and `RUN` for the first two rows (`tests/lg/hal_mock.sh`, LG's own test and library under
qemu-arm-static on the host, system calls traced):

| File | What it is |
|---|---|
| `libwebos_nnhal.so` (21,900 bytes, recipe `nnhal 2.0.1-1072-r4`) | A C++ API shaped like an Android NN HAL 1.2 driver, in namespace `android::nn::hal`; not a binder service. `getDevices()` returns one `IDevice`, `lgnpu::NpuNnHal`: name `lgnpu`, version `v1.0`, `getCapabilities_1_2`, `getSupportedOperations_1_2(Model)`, `prepareModel_1_2(Model, ExecutionPreference, Priority, cache files, token)`, `prepareModelFromCache`, and `NpuPreparedModel::executeSynchronously(Request, MeasureTiming, ...)`. It links libstdc++, libgcc_s and libc only; it imports no `open`, `ioctl`, `mmap`, `dlopen` or socket call and nothing of `libnpu`, `libnnrt` or the offline HAL, but `usleep`, `steady_clock::now` and `std::cout`. Its functions are 4 to 664 bytes (`validateModel` 12, `NpuPreparedModel::initialize` 4). |
| `lgnpu_online_nnhal_test` (292 KB, gtest) | One test, `NnHaTest.DeviceTest`: device name and version, feature level, extensions, capabilities, cache file counts, `getSupportedOperations_1_2`, `prepareModel_1_2`, `executeSynchronously(req, MeasureTiming::YES, results)` and `timing.timeOnDevice`. `RUN`: it passes in 6 ms and prints `count: 1` and `supported size: 1`; between two reads of the monotonic clock the only call is one `clock_nanosleep` of 1,000,000 ns; no file under `/dev` is opened, no `ioctl` (but the terminal query) and no socket are made. |
| `lgnpu_nn_hal_test` (276 KB, gtest) | The offline HAL's test (`libwebos_offline_nnhal`, `init`, `prepare`, `invoke`, `free`). It reads `./dne_custom_op.lgnpu`, a precompiled package, and reports "error: fail to open init data" without it. No such file is shipped. |
| `run_lgnpu` (186 KB) | A direct `libnpu` runner: `Usage: run_lgnpu [-i input tensor file] [-p .lgnpu file] [-o output file prefix] [-a affinity] [-c run count] -j [tensor meta json]`; tensor types fp32, int8, uint8, scale and zero point from the JSON, outputs written as `.bin`. It runs a package; it compiles nothing. |
| `nn-proxy` (75 KB) | `com.webos.service.nnproxy`, the AI picture and sound service over `libaibsp` and `libmali` (`SqmEnable`, `SREnable`, `GenreEnable`, "AI 8K Upscaling is Updated"). Not a model API. |

So the "online" HAL takes an NNAPI-style operand and operation graph and answers, but it neither compiles nor runs it: `prepareModel_1_2`
makes an object and `executeSynchronously` sleeps 1 ms. Nothing it imports could compute, so the outputs of a model run through it are
[INFERENCE] left as they were, while the time looks like a 1 ms NPU. That is why the probe's NNAPI phase compares outputs with the CPU's.
The rest of the firmware has no compile route either: no string in `libnpu`, `libnnrt`, `libwebos_offline_nnhal`, the delegate or `libaibsp` names
a model compiler, converter or graph builder. `libai-inference-interface` names `/model/tflite`, `/model/cache` and `/model/lib` and has
`installModel` [INFERENCE: models arrive already compiled, as IPKs]. A real online HAL would be a later firmware's; `tests/lg/hal_mock.sh`
pins this build's hash, and TV step 6a checks it on the set.

How an app would reach a real one: TFLite's NNAPI delegate, then `libneuralnetworks.so` (webosose's `webos-nnruntime`, pulled in by the
machine feature `webos-nnapi`; `SRC` `tensorflow-lite_2.9.3.inc`, whose `0007-enable-nnapi-delegate.patch` only drops the Android-only gate
from TFLite's CMake), then the HAL. TFLite 2.13.1 off Android derives the Android level from the functions the runtime exports (`SRC`
`CalculateAndroidSdkVersion`: 27 for NNAPI 1.0, 28 for 1.1, 29 for 1.2, 30 for 1.3), so a 1.2 runtime works and the delegate can target a device
by name (`accelerator_name=lgnpu`). Whether the set has `libneuralnetworks.so` is not in the copied files: TV step 1 lists it, step 6b loads it.

## ai-inference-manager (luna)

`JSON` `com.webos.service.aiinferencemanager`, binary `/usr/sbin/ai-inference-manager` (not copied), role type `regular`, `trustLevel oem`,
outbound `*`. Methods, in two permission groups:

| Group | Methods |
|---|---|
| `aiinferencemanager.query` | `getEvent` (subscribe), `getDeviceCapability`, `getDeviceStatus`, `getResourceList` |
| `aiinferencemanager` (all of those and) | `open`, `close`, `requestResource`, `releaseResource`, `installModel`, `handleModelInstall`, `createNotification` |

`SYM` Its client is `libai-inference-interface.so.1` (`AIInferenceInterface`: `startup`, `shutdown`, `requestResource`, `releaseResource`,
`createDetector/updateDetector/deleteDetector`, `detect*`, `pipe*`, `getCapableSolutionList`, `getDeviceCapability/Status`, `getResourceList`),
which calls `luna://com.webos.service.aiinferencemanager/{open,close,getEvent,requestResource,releaseResource,getDeviceCapability,
getDeviceStatus,getResourceList,installModel}`. Payload keys seen: `resourceKey`, `eventType`, `message`, `returnValue`, `subscribed`,
`detectors`, `modelList`, `mainDelegate`, `type`, `option`. Events: `RESOURCE_ALLOCATED`, `RESOURCE_ALLOC_FAILED`, `RESOURCE_RELEASED_FORCED`,
`RESOURCE_AVAILABLE`, `REQUEST_RESOURCE_RELEASE`, `MODEL_INSTALLED`, `MODEL_ERROR`. The detector types its client names are `FACE`,
`SIGNLANGUAGEAREA` and `TEXT`; the object-detection extension adds `BODY` (people, below). No animals.

Model paths: `/usr/share/aif/model/` (defaults, `<name>.tflite`), `/usr/share/aif/extra_models/`, `/usr/share/aif/model_caches/` (GPU delegate
serialisation), an installed model package's `/media/system/apps/usr/palm/applications/<ipk id>/model/{tflite,cache,lib}`,
`/usr/share/aiservice/*.tflite` with `default_models.json` (the DNN packages, `iSize`, `oSize`, `order`), `/usr/share/aibsp/*.dat`,
`/mnt/lg/cmn_data/edgeai/edgeai_extension_registry.json` and `/usr/lib/edgeai-extensions/*.so`.

The manager is the resource arbiter (a TOPS budget, `ARM::PriorityStrategy`, `AIModelTopsProvider`: `hardware-census.md`), but the delegate
path does not call it: `libtflite-npu-delegate` goes straight to `libnnrt` and `/dev/npu0`, and the driver has no lease call
(`hardware-census.md`: 13 ioctls). The detectors run in the calling process (`libai-inference-interface` loads `libedgeai-vision`); the manager
only grants and revokes resources. Nothing found makes LumaTV's own models reachable through it: model packages are LG-signed ipks with extension
libraries. LumaTV may call the four query methods (read only) to see the budget and holders before it opens the NPU (TV step 3); it does not call
`open`, `requestResource` or `installModel`.

## LG's own AI packages: what each computes, and what an app can call

`SYM` strings of `libaibsp`, `libaibspLib`, `nn-proxy`, `libai-inference-interface`, `libedgeai-vision-object-detection`, `run_lgnpu`; `JSON`; the
TFLite graphs read with the probe's `--info`. No disassembly, nothing run on the set.

| File | Kind | Computes | Loaded by | Callable by an app |
|---|---|---|---|---|
| `aibsp/SGNET_o22_text.dat` 14,336 B, `_param.dat` 694,272 B | program and int8 weights for the display engine's neural unit (`LNE` in the code) | **scene and genre** ("scene/genre fw/wgt"): eight int8 outputs dequantised with two scale and zero-point pairs, then a genre index and a face measure (`face_iir_measure`, `face_protect_th`), smoothed by an IIR filter (`/var/aibsp/ai_scene_iir_params.txt`); steers AI tone mapping (peak, histogram and tone-map parameters, face protection) | `libaibspLib` (`AIBSP_SCENE_*`): `LNE_IOCTL_INITIALIZE` on `/dev/lne_pqe_ipc`; the input is the picture the display engine presents | no |
| `aibsp/GNMNET_o22_text.dat` 17,408 B, `_param.dat` 370,688 B | the same kind | **grain and noise** management ("gnm") on a luma plane high-pass filtered by a GLES 3.1 compute shader; `net_en` switches it | `libaibspLib` (`AIBSP_GNM_*`) | no; not a scene type |
| `aiservice/DNN_GNET.tflite` 138,592 B | float32, 83 ops | **genre**: input [1,256,256,6], softmax [1,6]; run on the CPU by `libaibsp` (`DNN_GENRE_Inference`) on the display engine's down-scaled picture (`fpp L3`, letter-boxed) and merged with black-bar detection, the EPG genre and the audio genre into four genre slots (`genre_w_outidx`, `genre_w_blackbar`, `genre_w_epg`, `genre_w_audio` in its log lines) | `libaibsp`, inside `nn-proxy` | the result: yes, below |
| `aiservice/DNN_SQM1-3.tflite` 112,168 B each | float32, 21 ops | **source quality**: a 64x64 luma block (picked by a GLES Sobel pass [INFERENCE]), softmax [1,4]; selects the AI upscaler's weight set | `libaibsp` | `SqmGetData`, `SRGetData` through nn-proxy |
| `aiservice/sound_DNN.tflite` 810,300 B | 5 fully connected layers | 58 audio features a step to 60 outputs [INFERENCE: audio genre or a sound-mode control] | the audio service (`audio_DNN` in `default_models.json`) | its result enters nn-proxy through `GenreSetAudio`; nothing to read |
| `aif/model/FitTV_Detector_Yolov3_QAT_SoC.tflite` 4.39 MB | int8, builtin ops | **people** (solution `BODY`, `person_yolov3_qat`) | `libedgeai-vision-object-detection` | through `libai-inference-interface`; people only, CPU, 421 ms |

- **The two `.dat` pairs are not NPU packages in the `libnpu` sense.** The `_text.dat` files begin with a zero word where a `.lgnpu` flatbuffer has the offset of its
  root table (`_param.dat` with floats: `0, 1, 0, 1` for GNMNET), and `libaibspLib` hands them to a different device, `/dev/lne_pqe_ipc`, with a mailbox to the picture engine's firmware
  (`request2Mcu`, `response2Cpu`) and the frame buffers of `/proc/lg/de/fpp_initialize`. `run_lgnpu` runs only a package: `run_lgnpu [-i input tensor
  file] [-p .lgnpu file] [-o output file prefix] [-a affinity] [-c run count] -j tensor-meta.json`, the JSON naming `input_tensors` and
  `output_tensors` with `type` (fp32, int8, uint8), `shape`, `scale`, `zero`. No `.lgnpu` file ships, and no luna method returns what SGNET computes: it
  goes to a callback inside the process that loaded `libaibspLib` (TV step 6e finds that process). Whether the LNE and `/dev/npu0` are one block is not
  in the files; `npu_utilization` reads 0 at Home.
- **Person detection exists in LG's stack and is no use here.** `libedgeai-vision-object-detection` registers `BODY`: `FitTV_Detector_Yolov3_V1/V2`
  (NPU packages, `person_yolov3_v1_npu`, `_v2_npu`; not on the set) and `_QAT`, `_PTQ`, `_PTQ_RTK` (CPU; only QAT is on the set). People only, and 421 ms on the CPU.
- **nn-proxy** (`com.webos.service.nnproxy`) registers the category `/picture` with `set`, `get` and `debug` (three-letter names, below the default
  string length: the handlers are `OnSetPicture`, `OnGetPicture`, `OnDebug`). Its role file has `inbound ["*"]` and no API-permission file is among the
  copied ones [INFERENCE: any caller reaches it; TV step 6e calls it]. Payload keys `type`, `command`, `option`, `selected`, `subscribe`; `type` values
  `GENRE`, `SR_LEARN`, `SR_SQM`; commands `enable`, `disable`, `data`, `audio`, `demo`, `fast`, `period`. Genre calls: `GenreEnable`, `GenreEnableHalf`,
  `GenreDisable`, `GenreSetEPG`, `GenreSetAudio`, `GenreGetData`, `GenreGetDataSubscribe` (subscription key `GenreData`; fields `selected`, `data1` to
  `data4`); errors `disable`, `protected`, `invalid parameters`, `internal error`. LumaTV would only read (`get`, `command: data`); `enable` changes
  what LG's picture pipeline runs and is never called.

### Scene type for light sync

The ambient engine (`ambient/`) already shapes colours per light (`tuning.rs`: saturation, gain, dark gate; `smooth.rs`; `delay.rs`) from 16 zones at 10 polls a second.
The one scene-like signal LG hands out is the **genre** above: four slots from a network, black bars, the EPG and the audio, already computed for AI picture at
LG's cost. Proposal, for when TV step 6e shows `get` answering:

1. While ambient sync runs, the helper subscribes to `GenreData` (or polls `get` every 2 s), reads only, and never calls `enable`.
2. The genre index selects one of three tuning profiles (fast and vivid for live sport, slow and strict dark gate for film, default), changing at most once in 10 s;
   `disable`, `protected`, an error or 60 s of silence mean the default profile.
3. Which of the six classes is which genre is in no shipped file: step 6e logs `selected` and `data1` to `data4` for whatever plays, and the mapping is fixed
   from four viewings (sport, film, animation, game).
4. It is a hint, not a dependency: with AI picture's genre detection off the answer is `disable`, and the engine keeps its own cues (the zones' brightness and
   frame-to-frame change already separate fast from slow content). SGNET's scene output stays out of reach, and LG's `DNN_GNET` file is not run by LumaTV: its six
   input planes and their scaling are in `libaibsp`'s code, not in the strings.

## The GPU route: TFLite's GPU delegate on the Mali-G52

`SYM` `perf-tweaks.md`: Mali-G52 with 3 shader cores, kbase r1p0, Bifrost DDK r46p0 32-bit (`libmali.so.0`), no devfreq node; LumaTV's compositor loads the 64-bit blob
`libmali-bifrost-g52-g24p0-wayland-gbm.so`. LG's own picture code creates an EGL pbuffer context on that driver and dispatches GLES 3.1 compute shaders
(`#version 310 es`, `glDispatchCompute`, in `libaibsp` and `libaibspLib`), so GLES compute is part of this platform [INFERENCE: the C4 runs that code only with AI picture on]; OpenCL itself is not shown to work.

**OpenCL on the set (from the inventory).** `/usr/lib/libOpenCL.so.1.2` is the Khronos ICD loader (SONAME `libOpenCL.so.1`, OpenCL 3.0 symbol versions,
`clIcdGetPlatformIDsKHR`, EGL and GL interop entry points). It reads `/etc/OpenCL/vendors/*.icd`, the variables `OCL_ICD_VENDORS`, `OCL_ICD_FILENAMES` (driver
libraries named directly, no file) and `OPENCL_LAYERS`. `libOpenCLDriverStub.so` is, by its names (`test_icd_stub_log`, `cliIcdDispatchTableCreate`, a log call per `clGetDeviceIDs(...)` through `libIcdLog.so`),
the loader's own test driver, not a GPU. The vendor side, an `.icd` file or `libmali.so.0` exporting `clIcdGetPlatformIDsKHR`, is not in the copied files, so whether the loader finds a platform is open;
TV step 6c lists `/etc/OpenCL/vendors`, counts the OpenCL names in `libmali.so.0` and runs `--gpu-info`, and when the loader finds nothing it retries with
`--opencl-icd /usr/lib/libmali.so.0` (the loader's `OCL_ICD_FILENAMES`, this process only). TFLite's loader opens `libOpenCL.so` (the development name,
probably absent) and falls back to entry points already in the process; the probe opens the loader `RTLD_GLOBAL` first, which serves both. LG's `libedgeai-vision` has its own
`aif::GpuDelegate` (options `inference_preference`, `inference_priorities`, `enable_quantized_inference`, `backends`, `experimental_flags`,
`max_delegated_partitions`, `serialization` into `/usr/share/aif/model_caches/`) [INFERENCE: LG's face detector runs on it in some configurations].

**The probe's route** (`--gpu`, `--gpu-opt precision=fp32|fp16 quant=true|false pref=fast|sustained|balanced partitions=N backend=cl|gl|any cache=DIR`). The delegate
is TFLite 2.13.1's, OpenCL backend only, built into the probe: it needs nothing of the set's TFLite, only the loader. `fp16` sets `is_precision_loss_allowed` (the delegate
then asks for `MIN_LATENCY`: 16-bit arithmetic where `cl_khr_fp16` exists); `quant` keeps `ENABLE_QUANT` on, without which the int8 models run no node on the GPU
(every op "tensor type/shape isn't compatible", `QUANTIZE` "not supported"); `cache` turns on the delegate's serialisation (compiled kernels and tuning, keyed by a
hash of the model file), `pref` chooses start-up against sustained speed. The run prints the delegate's own coverage, the set-up time and the first invoke separately from the timed runs,
the outputs against the CPU's (`identical`, `close`, `zero` or `differ`), the process CPU share, and the Mali's busy share over the loop from LG's debugfs counters, with
the busy share of the idle second before it, from which the summary gives `gpu_ms_per_inference` (the GPU time one inference takes from everything else).

**Coverage** (`--gpu-coverage`: the delegate's own `GetOpsToReplace` over the real files, no device needed):

| Model | Nodes | On the GPU | On the CPU |
|---|---|---|---|
| EfficientDet-Lite0 int8 (`efficientdet_lite0_320_ptq`) | 267 | 266, one piece | `TFLite_Detection_PostProcess` |
| SSD-MobileNetV2 int8 (`tf2_ssd_mobilenet_v2_coco17_ptq`) | 102 | 101, one piece | `TFLite_Detection_PostProcess` |
| EfficientDet-Lite0 float32 (MediaPipe, `fetch-model.sh --gpu`) | 263 | all, one piece | none: raw box and class tensors, decoded in our code |
| SSD-MobileNetV2 float32 (MediaPipe) | 199 | all, one piece | none |

One piece each, so `partitions=1` loses nothing. The convolutions, depthwise convolutions, adds, max pools, nearest-neighbour resizes, concatenations, `LOGISTIC`, `QUANTIZE` and
`DEQUANTIZE` all go; only the post-processing op stays.

**Speed on the G52** `[INFERENCE]`, nothing measured on the set. The work is 1.95 GFLOP for Lite0 and 1.55 for SSD-MobileNetV2 (twice the MMAC in "Our model"). The G52 in the Amlogic S922X, measured
by Chips and Cheese, is two shader cores of three engines of eight lanes at 800 MHz: 24 FP32 FMA a clock a core, FP16 at double rate. The TV has three cores; the engines
per core and the clock are read at TV step 6c (`CL_DEVICE_MAX_COMPUTE_UNITS`, `CL_DEVICE_MAX_CLOCK_FREQUENCY`). With dual- to triple-engine cores at 600 to 800 MHz the FP16 peak is 115 to 230 GFLOPS, and TFLite's OpenCL kernels reach
12 to 30 % of it on detectors of this kind (depthwise convolutions and the BiFPN adds are bandwidth-bound). That gives **Lite0 30 to 140 ms and SSD-MobileNetV2 22 to 110 ms in FP16**
(central 65 and 50), about twice that in FP32. The CPU's 60 to 120 ms for Lite0 (below) is in the same range, so the GPU would free CPU cores, not make detection faster. Start-up is the other cost:
each distinct kernel is compiled by the driver, 100 or so for Lite0, an estimated 2 to 10 s cold; with `cache=DIR` the second start skips it (steps 6d: cold and warm). The CPU path starts in under 100 ms.

**Cost to the compositor's frame budget.** The Mali has little spare while the UI animates: the Rust client's frames take 12.9 ms (p50) to 16.9 ms (p90) of GPU against a 16.7 ms
period with the flip pass (`rustui-result.md`), 66 to 71 % busy at 54 fps; LG's QML frames 11 to 13.5 ms. At Home the idle UI keeps it 10 % (QML) to 23 % (Rust, 6.2 ms a 10 fps frame) busy, and
LumaTV's compositor alone 0.5 ms a frame at 10 fps (`compositor-c2-result.md`). A GPU inference is one queue of 100 or so kernels and one wait: it cannot be paused halfway. A 30 to 140 ms job
during an animation therefore costs 2 to 8 frames; at Home, at most one 10 fps frame. Rules for any detector, GPU or CPU: (1) it runs only while a camera pop-up or camera view needs it,
never as a background sweep, one inference in flight, at most one in 2 s a camera; (2) it does not start while a panel or an app has asked for the frame budget (the helper refuses the request,
as the pop-up section below already has it), and a view with a video plane has a nearly static UI, which is when it runs; (3) CPU: `nice 19` and two threads; GPU: the queue at low priority when `--gpu-info`
lists `cl_khr_priority_hints`: TFLite creates its queue with `clCreateCommandQueue(context, device, 0)` (`delegates/gpu/cl/cl_command_queue.cc`, `CreateCLCommandQueue`), so low priority is one patch there
(`clCreateCommandQueueWithProperties` with `CL_QUEUE_PRIORITY_KHR`, `CL_QUEUE_PRIORITY_LOW_KHR`), not made because the CPU path is the default. TV step 6d measures the other half: the
page-flip cadence (`/proc/lg/fbdev/fb0/flip_cnt`) and the Mali's busy share before, during and after a 300-run GPU loop and the same loop on the CPU.

## Decision

1. Detection for the camera pop-ups runs on the CPU with XNNPACK, in a 32-bit worker (below). It does not need the NPU: the pop-up path
   takes a frame at most every few seconds per camera.
2. The NPU entry is kept in the probe and the worker (an option, off): it works the day an NPU package exists, with nothing else to build.
3. Getting our own network onto the NPU needs LG's offline compiler or the package and DNE instruction formats. Neither is on the set or
   published. Reading the formats goes past symbols and strings, the limit set for this work, so it is Main's decision whether to start.
4. The NNAPI route is kept in the probe (`--nnapi`) and checked on the set (TV step 6b), but nothing in the firmware gives it something to
   compile with: the HAL behind it is a 1 ms stand-in.
5. The GPU is the one accelerator that needs no compiler, and the probe has it (`--gpu`): the delegate takes both detectors whole but the post-processing op. It stays a
   measured option, not the default: the estimate (30 to 140 ms) is no better than the CPU's, the GPU is shared with a UI that keeps it 10 to 70 % busy, a cold start costs seconds
   of kernel build, and a 30 to 140 ms job cannot be paused for an animation. It becomes the worker's `backend=gpu` only if step 6d shows FP16 at 40 ms or less per frame
   with the cache warm and a start under 1 s, and the page-flip cadence at Home unchanged (no late flips) while it runs; the frame-budget rules of "The GPU route" apply to it as to the CPU.
6. LG's own AI programs are not called (`SGNET`, `GNMNET`: no interface; `BODY`: slower and people only). LumaTV reads one thing from LG's AI picture, the genre from nn-proxy
   (`get` only), as a hint for the ambient engine's tuning profile; it never calls `set`/`enable`.

## Our model

`npuprobe/fetch-model.sh` fetches **EfficientDet-Lite0, full-integer quantised, 320x320** (`efficientdet_lite0_320_ptq.tflite`, 4.56 MB,
sha256 `33a3b622…5396704`) and `coco_labels.txt` from `github.com/google-coral/test_data` at commit `104342d2…` (repository licence
Apache-2.0; the network is Google's TF Model Maker EfficientDet-Lite, Apache-2.0, trained on COCO 2017). Into `/tmp/lumatv-npu-models`,
not committed.

It is quantised the way LG's QAT detector is: uint8 input, `QUANTIZE`, int8 activations, per-channel int8 weights, no fp32 inside the
network. Its post-processing is in the graph (`TFLite_Detection_PostProcess`: boxes, classes, scores, count, at most 25), so the output
decode is a few lines. COCO ids 0 to 89: person 0; animals bird 15, cat 16, dog 17, horse 18, sheep 19, cow 20, elephant 21, bear 22,
zebra 23, giraffe 24; vehicles bicycle 1, car 2, motorcycle 3, bus 5, truck 7.

| | LG QAT detector | EfficientDet-Lite0 int8 | SSD-MobileNetV2 int8 (TF2 PTQ) |
|---|---|---|---|
| input | 270x480 uint8 | 320x320 uint8 | 300x300 uint8 |
| MMAC | 1,264 | 974 | 773 |
| ops | QUANTIZE PAD CONV_2D DEPTHWISE_CONV_2D ADD RESIZE_NEAREST_NEIGHBOR CONCATENATION | CONV_2D DEPTHWISE_CONV_2D ADD MAX_POOL_2D RESIZE_NEAREST_NEIGHBOR RESHAPE CONCATENATION LOGISTIC QUANTIZE DEQUANTIZE, `TFLite_Detection_PostProcess` | CONV_2D DEPTHWISE_CONV_2D ADD RESHAPE CONCATENATION LOGISTIC QUANTIZE DEQUANTIZE, post-process |
| outside the ops LG's shipped models use | n/a | `DEQUANTIZE` and the CPU post-process only | the same |
| x86 host, XNNPACK, 4 threads | 18.9 ms | 19.5 ms | 9 to 11 ms |
| x86 host, builtin kernels, 4 threads | 128 ms | 111 ms | not run |

The convolution graph of Lite0 is a subset of LG's shipped op set (LG's models together use ADD, AVERAGE_POOL_2D, CONCATENATION, CONV_2D,
DEPTHWISE_CONV_2D, FULLY_CONNECTED, LOGISTIC, MAX_POOL_2D, MUL, PAD, QUANTIZE, RESHAPE, RESIZE_NEAREST_NEIGHBOR, SOFTMAX, STRIDED_SLICE,
TRANSPOSE); `DEQUANTIZE` and the post-process op would stay on the CPU in an NPU build. Lite0 over SSD-MobileNetV2: higher COCO accuracy
(mAP about 25.7 against about 20 [INFERENCE: figures from the model cards, not measured here]), which matters for small people and animals far
from a camera, and the feature pyramid with `RESIZE_NEAREST_NEIGHBOR` and per-channel int8 convolutions that LG's own detector has. SSD-MobileNetV2
is about twice as fast on the host and is the fallback if Lite0 is too slow on the TV (`fetch-model.sh --all`, TV step 4 times it too). The probe
finds seven people in LG's own `5people.jpg` and one in `person.jpg` (`tests/run_tests.sh`).

**Expected speed on the TV** (`[INFERENCE]`, TV step 4 measures it). `TV` LG's detector takes 421 ms on the CPU with 4 threads (`PLAN.md`,
builtin kernels, 1,264 MMAC). On the host Lite0 takes 0.87 of LG's time with builtin kernels and 1.03 with XNNPACK, and XNNPACK is 6.8 times
faster than builtin kernels for LG's model. Scaled: about 370 ms with builtin kernels and 4 threads; with XNNPACK about 60 to 70 ms with 4 threads
and 80 to 120 ms on the 3 cores LG leaves online at Home. Arithmetic for the NPU, if a package existed: 974 MMAC at 0.2 TMAC/s is 4.9 ms at full
rate, 16 to 20 ms at the 25 to 30 % utilisation `hardware-census.md` assumes.

A second anchor for the CPU: Google lists 37 ms for EfficientDet-Lite0 int8 on a Pixel 4 with 4 threads on the CPU (TensorFlow Lite Model Maker page, "Latency measured on
Pixel 4 using 4 threads on CPU"). That phone's cores are A76-class at about 2.3 GHz; the same code at 1.4 GHz is 61 ms on four cores and 81 ms on three. The range above
(60 to 120 ms) holds both anchors.

## The probe

`npuprobe/` (usage is in the header of `build.sh` and in `lumatv-npu-probe --help`; nothing is installed on the system):

- `build.sh [arm|host|all]` unpacks the toolchains (`toolchain.py`: Ubuntu 22.04 GCC 11.4 and glibc 2.35 cross for `arm-linux-gnueabi`, Debian 12
  binutils, GCC 12 for the host, cmake 3.27, ninja, qemu-arm-static) under `/tmp/lumatv-npu-tools`, fetches TensorFlow 2.13.1 and `stb_image.h`
  (pinned, checked), patches TFLite's CMake so that its NNAPI delegate builds on Linux (`patch_tflite.py`: the change webosose's
  `0007-enable-nnapi-delegate.patch` makes), builds TFLite with XNNPACK, NNAPI and the GPU delegate (OpenCL backend only) and its dependencies for each target, then `lumatv-npu-probe`,
  `libstub-npu-delegate.so`, `libstub-gpu-delegate.so`, `opencl-stub/libOpenCL.so.1` (a stand-in ICD loader), `nnapi-stub/libneuralnetworks.so` and `nnapi-stub-12/libneuralnetworks.so` (stub NNAPI runtimes), `libnot-a-delegate.so`,
  `make-test-models` into `build/{arm,host}/`. The ARM binary is `e_flags 0x5000200`, interpreter
  `/lib/ld-linux.so.3`, needs only GLIBC up to 2.34 and `libgcc_s` (libstdc++ is linked in because the set's stops at GLIBCXX_3.4.29).
  GCC 12 or zig cannot be used for the ARM build: GCC 12's libstdc++ adds GLIBCXX_3.4.30 and `arc4random`, zig cannot emit NEON for the
  soft-float ABI.
- `lumatv-npu-probe --model M [--image I] [--runs N] [--threads T] [--delegate LIB --delegate-opt K=V ...]`: a CPU phase (XNNPACK or builtin
  kernels), then a delegate phase. Per inference milliseconds (all of them, then min, mean, median, p95, max), process CPU time and share of a core,
  `/proc/npu/npu_utilization` sampled every 5 ms during each phase and `npu_process_list` before and after, which nodes the delegate took (ops
  before and after, delegate kernels, the list of absorbed ops), the outputs compared with the CPU's, the top detections with COCO names, and a
  `SUMMARY` line of key=value pairs. `--info` describes a model (and reports `lgnpu_custom_op` nodes and their package size); `--system-info`
  reads the device's TFLite version, XNNPACK, NNAPI runtime and the delegate's entry points without running anything of LG's; `--fill const:V`,
  `--norm`, `--labels`, `--min-score`, `--top`, `--print-output`, `--require-delegate` (exit 3 unless a kernel was installed and the runs
  completed, 5 if the outputs are not the CPU's).
  `--nnapi` takes the place of `--delegate`: the same second phase through TFLite's NNAPI delegate over the device's `libneuralnetworks.so`
  (`--nnapi-opt accelerator=lgnpu`, `sdk=N`, `pref=low|fast|sustained`, `partitions=N`, `allow_cpu=true`, `allow_fp16=true`), listing the runtime's
  devices first and then which ops landed on the device and the time. A device is only believed if its outputs match: they are compared with the
  CPU's as `identical`, `close` (within 2 % of the largest value, a quantisation step or two), `zero` or `differ`, the last two with a WARNING
  (`nnapi_outputs=` in `SUMMARY`). `--nnapi-list` lists the runtime's devices and exits. TFLite asks every selected device for its NNAPI 1.3
  feature level without checking that the runtime has the call; the probe supplies one for a 1.2 runtime.
  A crash inside the plugin prints the module, offset and symbol (the set has no debugger); an exception from the create call is caught.
  `--gpu` takes the place of `--delegate` and `--nnapi`: the same second phase through TFLite's GPU delegate V2 on OpenCL, the one built into the probe or one
  taken from a library (`--gpu-lib`, which must export `TfLiteGpuDelegateV2Create`, `...Delete` and `TfLiteGpuDelegateOptionsV2Default`). `--gpu-opt`:
  `precision=fp32|fp16`, `quant=true|false`, `pref=fast|sustained|balanced`, `partitions=N`, `backend=cl|gl|any`, `cache=DIR`. `--gpu-info` lists what the set's
  OpenCL loader offers: the `.icd` files and whether their libraries load, the platforms, and per device the compute units, clock, memory, `cl_khr_fp16`, the Arm
  int8 dot product, `cl_khr_priority_hints`, and whether the Mali counters are readable (exit 3 when no device is found); `--opencl-lib` names the loader to open,
  `--opencl-icd` a vendor library for the loader to use without an `.icd` file (`OCL_ICD_FILENAMES`, this process only). `--gpu-coverage` runs the delegate's own
  node selection on the model (no device needed) and prints which nodes it takes, the pieces they form and TFLite's reason for each op it leaves. A `--gpu` run prints
  that coverage, the Mali's busy share with nothing of the probe's running, the set-up time and the first invoke separately from the timed runs, the Mali's busy share
  over the loop (LG's debugfs counters; `--gpu-proc`), and in `SUMMARY` `gpu_init_ms`, `gpu_first_ms`, `gpu_mali_pct`, `mali_idle_pct` and `gpu_ms_per_inference`.
  The outputs are compared with the CPU's tensor by tensor; a detector's rows are sorted by score, so when the raw tensors differ the decoded detections are compared
  (same class, boxes overlapping by half, scores within 0.15) and `close` is the verdict when they agree. With no node taken the comparison raises no warning.
- `tests/run_tests.sh [host|arm]`: 100 checks on the host build and 99 on the ARM one against `stub/stub_delegate.cc` (LG's plugin surface: options, custom-op-only claim rule, refusal
  text, `use_custom_alloc` through the exported `SetCustomAllocationForTensor`, a configurable device delay) and `stub/stub_nnapi.cc` (a stand-in
  NNAPI runtime with the devices `lgnpu-stub`, an accelerator, and `nnapi-reference`, a CPU; it computes float32 ADD, so data crossing between CPU
  and device can be checked, and accepts every other operation it claims without computing it, as LG's HAL does; switches `STUB_NNAPI_OPS`,
  `_DEVICES`, `_DELAY_US`, `_FAIL=compile|compute`, `_LOG`; a second build leaves out the 1.3 feature-level call), `stub/stub_gpu_delegate.cc` (TFLite's three GPU
  delegate entry points with the real options struct: it takes the largest `partitions` pieces of the ops it supports (`STUB_GPU_OPS`), computes by running the model
  in a nested interpreter, or zeroes the outputs (`STUB_GPU_COMPUTE=none`), fails to create or to prepare, delays, and logs the options it received) and `stub/stub_opencl.cc`
  (a loader with one Mali-G52-shaped device) and generated models, among them one with exactly LG's op set (`tests/make_models.cc`), one with a lone `lgnpu_custom_op` and three
  detector-shaped ones, and the real detector on LG's sample pictures. The NNAPI checks cover device listing and selection, partial and full claims, a device that computes
  nothing (exit 5), failing compilation and execution, a missing runtime, and which of LG's op set TFLite hands an NNAPI 1.2 device (all but `QUANTIZE` and
  `RESIZE_NEAREST_NEIGHBOR`). The GPU checks cover the options reaching the delegate (precision, preference, partitions, quant, backend, cache directory and token), one and two
  pieces, a delegate that computes nothing, takes nothing, cannot start or cannot be created, detectors that come back reordered (close) or short (exit 5), the Mali counters from a
  fake file, a missing or wrong library, the built-in delegate refusing the graph where the loader has no driver, the real selection code on the generated models and on both detectors, `--gpu-info` with and
  without a loader, an `.icd` directory and `--opencl-icd`, and the option parser. `tests/test_ui_impact.py` (5 checks) runs `gpu-ui-impact.py` on fake counter files.
  Passing on both builds (the ARM one under qemu-arm-static).
- `tests/lg/run.sh` (needs the copied delegate): the 18 checks of the entry section, on LG's real binary.
- `tests/lg/hal_mock.sh` (needs the copied HAL files): the 19 checks of the section on the online NN HAL, on LG's real test binary and library.

## TV slot

Needs Main's slot; read only except for CPU load, GPU load with `--gpu-run`, and files under `/tmp/lumatv-npu`. The set at Home, awake. About 20 minutes with 100 runs (the builtin-kernel runs dominate; `--runs 30` takes about 6); `--gpu-run` adds about 10.
Push (from the workstation, with the repo's `docs/research/arch/tv.py`):

```
python3 - <<'EOF'
import sys; sys.path.insert(0, "docs/research/arch")
import tv
tv.tv("mkdir -p /tmp/lumatv-npu/models")
tv.put("npuprobe/build/arm/lumatv-npu-probe", "/tmp/lumatv-npu/lumatv-npu-probe")
tv.put("npuprobe/tv-probe.sh", "/tmp/lumatv-npu/tv-probe.sh")
for f in ("efficientdet_lite0_320_ptq.tflite", "coco_labels.txt", "tf2_ssd_mobilenet_v2_coco17_ptq.tflite",
          "mp_efficientdet_lite0_float32.tflite", "mp_ssd_mobilenet_v2_float32.tflite"):   # fetch-model.sh --all --gpu first
    tv.put("/tmp/lumatv-npu-models/" + f, "/tmp/lumatv-npu/models/" + f)
tv.put("npuprobe/gpu-ui-impact.py", "/tmp/lumatv-npu/gpu-ui-impact.py")
tv.tv("chmod +x /tmp/lumatv-npu/lumatv-npu-probe")
print(tv.tv("sh /tmp/lumatv-npu/tv-probe.sh --runs 100 --threads 3 --gpu-run", timeout=2400))   # in a shell with a PTY, so that luna-send answers; add --no-luna without one
EOF
```

Pull `/tmp/lumatv-npu/results/*` with `tv.get`, then `tv.tv("rm -rf /tmp/lumatv-npu")`.

| Step | What | Expected | Says |
|---|---|---|---|
| 0 | `/proc/npu/*`, `/dev/npu0` holders, CPU online and frequency | `npu_utilization` 0, empty `npu_process_list`, no holder, `aipq compatibility : false` (printed to the summary); the script stops if something holds the NPU (`--force` to go on) | nothing else (the AI picture path, an app) holds the NPU; if `ai_pq` is on, `aipq` and the process list change and the NPU is shared |
| 1 | `lumatv-npu-probe --system-info` and `ls -l` of the libraries | `device runtime: libtensorflow-lite.so, TfLiteVersion() = 2.13.x`, `ABI: ... same minor release`, entry points present, `libXNNPACK.so` loadable | the ABI gate for steps 5 and 6; any other version: do not run them, rebuild the probe with that release (`TFLITE_VERSION`, `TFLITE_SHA256`) |
| 2 | every `*.tflite` under `/usr/share /usr/lib /mnt/lg /media /var /opt`, `grep lgnpu_custom_op` | `0 of N carry lgnpu_custom_op` | whether any NPU package exists to run; the whole route hangs on this |
| 3 | by hand, in a shell with a PTY (`luna-send` hangs without one; run the script without `--no-luna` there and it does this itself): `luna-send -n 1 -f luna://com.webos.service.aiinferencemanager/{getDeviceCapability,getDeviceStatus,getResourceList} '{}'`, the query group, read only | JSON with the NPU's tops (0.4), the holders and the registered models, or a permission error | the real field names, `aipqCompatibility`, and the free budget |
| 4 | LG's detector and ours (and SSD-MobileNetV2 when pushed), `xnnpack` and `builtin`, 1 and 3 threads, 100 runs, `/usr/share/aif/images/people.jpg` | ours, XNNPACK, 3 threads: 80 to 120 ms median; builtin, 4-thread class: about 370 ms; LG's within a factor of 1.3 of ours; `person` x5 for `people.jpg` | the CPU path's real cost and its CPU share (`cpu_busy_pct`, about 300 for 3 threads) |
| 5 | LG's delegate on both models, `min_freq=30`, `--cpu-backend none` | both `delegate NOT APPLIED`, no crash; the 100 timed runs are the CPU path with builtin kernels (the delegate took nothing), `delegate_npu_util_max=0`; `grep NPUDELEGATE /var/log/messages` shows `Nodes doesn't have customOp node for NPU`; `npu_process_list` stays empty | the refusal on the set itself; a crash line means the TFLite release differs |
| 6 (`--npu-run`) | each model found in step 2, `--fill const:0`, delegate on | `delegate kernel(s) took 1 node`, `npu_utilization` non-zero during the loop, `npu_process_list` names the probe | the NPU's real speed and utilisation; skipped when step 2 found nothing |
| 6a | `sha256sum` of the HAL libraries, LG's own `lgnpu_online_nnhal_test` (only if `libwebos_nnhal.so` is the analysed build), `run_lgnpu -h`; with `--npu-run` also `lgnpu_nn_hal_test` in an empty directory | `lgnpu_online_nnhal_test` prints `[  PASSED  ] 1 test.` and `count: 1supported size: 1` in about 6 ms; `run_lgnpu -h` prints its usage; the hash is `963204be...`; empty process list after | that the set's HAL is the stand-in analysed here; a different hash means a later HAL and the analysis starts again |
| 6b (`--nnapi-run`) | `lumatv-npu-probe --nnapi-list`, then our model through `--nnapi --nnapi-opt accelerator=lgnpu` (partitions 3 and 16), then LG's `benchmark_model --use_nnapi=true --nnapi_accelerator_name=lgnpu`; utilisation sampled | `libneuralnetworks.so` loaded with the device `lgnpu` (or `libneuralnetworks.so is not loadable`: the set has no NNAPI runtime); if loaded: ops delegated, about 1 ms per inference, `nnapi_outputs=zero` or `differ`, `npu_util` 0 [INFERENCE: the stand-in] | which ops would land on the NPU and what it would take; a time with `nnapi_outputs=identical` would be real |
| 6c | read only: `ls -l` of the OpenCL and Mali libraries and `/etc/OpenCL/vendors`, the `.icd` files, how many lines of `libmali.so.0` and `libOpenCL.so.1` name `clGetPlatformIDs`, `clIcdGetPlatformIDsKHR`, `cl_khr_fp16`, `cl_khr_priority_hints`, the Mali driver's `gpuinfo`, `power_policy`, `core_mask`, `js_scheduling_period`; `lumatv-npu-probe --gpu-info` (and again with `--opencl-icd /usr/lib/libmali.so.0` if the first finds no platform); `--gpu-coverage` of each pushed model | a platform `ARM Platform` with the device `Mali-G52` of 3 compute units, `fp16 arithmetic yes`, perhaps `priority hints yes`; or `no OpenCL platform (-1001)` and then whether the retry with the Mali library finds one; coverage 266 of 267 and 101 of 102 for the int8 files, all of the float32 ones, as on the host | whether OpenCL works on the set at all and by which path (an `.icd` file, or `OCL_ICD_FILENAMES`), and the cores, clock and extensions the speed estimate needs |
| 6d (`--gpu-run`) | our int8 model in FP16 with the kernel cache cold, then warm, then FP32, then the short-start preference; the float32 Lite0 in FP16 (cold, warm) and FP32; the SSD-MobileNetV2 int8 and float32; each with a CPU phase in the same run; then `gpu-ui-impact.py` around a 300-run GPU loop and a 300-run CPU loop: flips per second and gaps of `/proc/lg/fbdev/fb0/flip_cnt` and the Mali's busy share before, during and after | `gpu_applied=1`, `gpu_outputs=close` (or `identical`), Lite0 FP16 30 to 140 ms, `gpu_init_ms` cold 2 to 10 s and warm under 1 s [INFERENCE], `gpu_ms_per_inference` close to the loop's mean; at Home the flip rate near 12 a second before, during and after, no `late` flips for the CPU loop, a few for the GPU loop if the job overlaps a frame | the GPU's real speed against the CPU's in the same run, the kernel cache's gain, FP16 against FP32, whether a detection loop disturbs the UI's cadence, and so whether `backend=gpu` is worth having (Decision 5) |
| 6e | read only: processes that map `libaibspLib`, `libaibsp`, `libnpu` or the vision libraries, the holders of `/dev/lne*`, `/dev/npu0` and `/dev/mali0`, `/proc/devices`, `/var/aibsp` and the small parameter files in it, EdgeAI's extension registry, `airunner --help`; with luna: `luna-send -n 1 -f luna://com.webos.service.nnproxy/picture/get` with `{"type":"GENRE","command":"data"}`, `SR_SQM` and `SR_LEARN` the same, `{"type":"GENRE"}`, and the `GenreData` subscription for 8 s | the picture-quality service (the process that holds `/dev/lne_pqe_ipc`) maps `libaibspLib`, `nn-proxy` maps `libaibsp`; the registry lists the solutions (`BODY` among them); `get` answers with `selected` and `data1` to `data4` while AI picture's genre detection is on, or `disable`/`protected`/`invalid parameters` [INFERENCE] | which process runs SGNET and GNMNET, whether `/picture/get` is reachable by us and what it returns, the keys to build the light-sync hint on (Decision 6); play a sports clip, a film, an animation and a game in turn to fix the index to genre mapping |
| 7 | state after, `dmesg` lines about `npu` | empty process list, no `npu` error lines | nothing left held |

`tv-probe.sh` writes `00-before.txt` to `07-after.txt` and `summary.txt` (one line per step) under `/tmp/lumatv-npu/results`.

Rollback: nothing outside `/tmp/lumatv-npu` changes; `rm -rf /tmp/lumatv-npu`. The probe holds `/dev/npu0` only inside steps 6 and 6b (in 6b only if the HAL is not the stand-in). The script
prints the `aipq compatibility` line at step 0 and again at step 6b: `false` means the NPU is not shared with AI picture (`ai_pq`), so if AI picture
is on and the picture changes during a run, stop the probe; switching AI picture back on is a person's action in the TV's settings. If a
run hangs, kill the probe (`kill -9`): the driver keeps one context per open file [INFERENCE: `npu_drv.c` per-open `inst_id`, `hardware-census.md`], so closing the
descriptor frees it; the status files read as in step 0. A reboot is infrastructure and Nitin's decision (Principle 0). Step 6d (`--gpu-run`) is the only one that loads
the Mali: one process of ours, the probe, which opens `/dev/mali0` through the loader and nothing else; if the UI stutters or the picture glitches, kill it (`kill -9`), and the driver
frees the GPU context with the file. Its kernel cache and everything else are under `/tmp/lumatv-npu`. The slot sets no LG setting; luna is called only for the query group of the
AI inference manager (step 3) and `get` of nn-proxy (step 6e), never `set`, `enable` or a model install.

## How the camera pop-ups would call it

`ha-cameras-result.md`: Home Assistant's own detectors (Reolink, Scrypted, Kestrel) already send `person`, `vehicle`, `animal` events; the
helper's `cameras.rs` turns them into `cameras.popup`, and `GET /v1/cameras/{id}/frame?w=&h=` serves the camera's current picture. The TV's
detector would cover the cameras that report no kind, and confirm or drop a plain `motion` event. Shape, nothing of it written yet:

- **Worker.** `lumatv-detect`, a 32-bit process the helper starts with a connected socket as fd 0 (as `lumatv-jpeg`), holding the model
  resident (about 30 MB). One request in flight. Protocol like `jpeg/jpegproto.h`: request `op=DETECT`, `format` (RGB888 or BGRA), `w`, `h`,
  `min_score`, the pixels; reply `status`, `backend` (`cpu`, `gpu` or `npu`), `ms`, `n`, then `n` records `{class, score, ymin, xmin, ymax, xmax}` in 0..1
  of the picture. It `nice`s itself; the helper never has two requests outstanding and refuses one while a panel or an app asked for the frame
  budget.
- **Helper route.** `GET /v1/cameras/{id}/detect?minScore=` runs the worker on the picture the frame route already holds (`frameHoldMs`) and
  answers `{"camera","at","ms","backend","detections":[{"kind":"person","label":"person","score":0.91,"box":[y0,x0,y1,x1]}]}`. `kind` is the pop-up
  vocabulary: person (id 0), vehicle (1, 2, 3, 5, 7), animal (15 to 24); anything else is left out. The same result goes to the hub topic
  `cameras.detect` (latest per camera) and, when a kind is in `cameras.popupKinds` and the cooldown allows, becomes an event through the
  existing path (`POST /v1/cameras/event` with `source:"lumatv-detect"`), so the pop-up policy, cooldown and clearing stay as they are.
- **Config.** `cameras.detect.enabled` (false), `intervalSeconds` per camera without a detector (0: only on a motion event), `minScore` (0.5),
  `threads` (2), `backend` (`cpu`; `gpu` only if step 6d meets Decision 5; `npu` only once a package exists). Off: no process, no model load, no fetch.
- **Budget.** At 60 to 120 ms a frame on 3 threads (about 0.3 core-seconds), a dozen cameras sampled every 10 s cost about 0.36 of a core, 12 % of
  the three at Home [INFERENCE from the expected speed]; run only while a pop-up or camera view is up, one request in flight, at `nice 19`, and not while a panel or an app has
  asked for the frame budget (the rules in "The GPU route").
