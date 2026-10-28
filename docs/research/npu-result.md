# NPU: the entry LG ships, what it accepts, and what LumaTV runs

## Resume here

**Result.** LG's NPU is reached through the public TFLite external-delegate plugin `tflite_plugin_create_delegate` in
`/usr/lib/libtflite-npu-delegate.so.1`. The plugin takes only a graph that contains LG's own node, the custom op `lgnpu_custom_op`,
whose `custom_options` are an NPU package compiled offline by LG. It refuses every graph of builtin ops, including LG's shipped
`FitTV_Detector_Yolov3_QAT_SoC.tflite` (builtin ops only). No compiler or package format is published, so a public int8 detector cannot
run on the NPU through this entry. Person and animal detection for the camera pop-ups therefore runs on the CPU (XNNPACK): the model
is EfficientDet-Lite0 int8, below. The NPU entry stays wired in the probe for the day a package turns up (TV slot step 2 looks).
This is the reason `PLAN.md` saw the delegate "apply to none of LG's shipped models": it was never going to take a builtin-op graph.
The other route, a standard model through NNAPI to LG's "online" NN HAL (`libwebos_nnhal.so`, `lgnpu_online_nnhal_test`), is no route either:
that HAL is a stand-in whose execute call sleeps 1 ms and reports it as the time on the device; nothing in the firmware compiles a model for
the NPU (section "The online NN HAL"). The probe has the NNAPI path anyway (`--nnapi`), with a check that catches a device that does not compute.

**Done offline (2026-10-28), nothing run on the TV.**

- Symbols, strings and JSON of the 24 ELF files and 7 JSON files copied off the set (sizes and hashes in the copy's `inventory.txt`,
  scratch under `/tmp/lumatv-npu-tv`). No disassembly.
- LG's delegate run as a black box on the host (`tests/lg/run.sh`, 18 checks): its options, the graphs it takes and refuses, the calls it makes
  into `libnnrt`, and the TFLite structure layout it was built against (2.13). LG's online NN HAL test run the same way (`tests/lg/hal_mock.sh`,
  19 checks, system calls traced under qemu-arm-static): what the HAL does and does not touch.
- `npuprobe/`: `lumatv-npu-probe` (TFLite 2.13.1 static, 32-bit ARM soft-float and x86-64), a stub delegate with LG's plugin surface,
  a stub NNAPI runtime for its NNAPI path, 50 tests that pass on both builds (the ARM one under qemu), and the TV slot script `tv-probe.sh`.
- The model, `npuprobe/fetch-model.sh`.

**Open (needs a TV slot, below).** The device's own TFLite version (the delegate is only safe in a process with the same 2.13 layout),
whether any `.tflite` on the set carries an NPU package, the speed of the model on the A76s, and whether the set has `libneuralnetworks.so`
(the NNAPI runtime; it would change nothing about speed while the HAL behind it is the stand-in).

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
`RESOURCE_AVAILABLE`, `REQUEST_RESOURCE_RELEASE`, `MODEL_INSTALLED`, `MODEL_ERROR`. The detector types it knows are `FACE`, `SIGNLANGUAGEAREA`
and `TEXT`: no person or animal.

Model paths: `/usr/share/aif/model/` (defaults, `<name>.tflite`), `/usr/share/aif/extra_models/`, `/usr/share/aif/model_caches/` (GPU delegate
serialisation), an installed model package's `/media/system/apps/usr/palm/applications/<ipk id>/model/{tflite,cache,lib}`,
`/usr/share/aiservice/*.tflite` with `default_models.json` (the DNN packages, `iSize`, `oSize`, `order`), `/usr/share/aibsp/*.dat`,
`/mnt/lg/cmn_data/edgeai/edgeai_extension_registry.json` and `/usr/lib/edgeai-extensions/*.so`.

The manager is the resource arbiter (a TOPS budget, `ARM::PriorityStrategy`, `AIModelTopsProvider`: `hardware-census.md`), but the delegate
path does not call it: `libtflite-npu-delegate` goes straight to `libnnrt` and `/dev/npu0`, and the driver has no lease call
(`hardware-census.md`: 13 ioctls). Nothing found makes LumaTV's own models reachable through the manager: it serves FACE, sign-language and
text detectors, and model packages are LG-signed ipks with extension libraries. LumaTV may call the four query methods (read only) to see the
budget and holders before it opens the NPU (TV step 3); it does not call `open`, `requestResource` or `installModel`.

## Decision

1. Detection for the camera pop-ups runs on the CPU with XNNPACK, in a 32-bit worker (below). It does not need the NPU: the pop-up path
   takes a frame at most every few seconds per camera.
2. The NPU entry is kept in the probe and the worker (an option, off): it works the day an NPU package exists, with nothing else to build.
3. Getting our own network onto the NPU needs LG's offline compiler or the package and DNE instruction formats. Neither is on the set or
   published. Reading the formats goes past symbols and strings, the limit set for this work, so it is Main's decision whether to start.
4. The NNAPI route is kept in the probe (`--nnapi`) and checked on the set (TV step 6b), but nothing in the firmware gives it something to
   compile with: the HAL behind it is a 1 ms stand-in.
5. A second accelerator exists in LG's own stack: the TFLite GPU delegate (`libedgeai-vision` imports `TfLiteGpuDelegateV2Create`; the
   recipe's `webos-gpu-delegate` feature and its OpenCL wrapper patches; `auto_delegation_test` has GPU-cache cases). It needs no compiler.
   Not probed here; a `--gpu` mode is the next extension if the CPU's 80 to 120 ms is not enough.

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

## The probe

`npuprobe/` (usage is in the header of `build.sh` and in `lumatv-npu-probe --help`; nothing is installed on the system):

- `build.sh [arm|host|all]` unpacks the toolchains (`toolchain.py`: Ubuntu 22.04 GCC 11.4 and glibc 2.35 cross for `arm-linux-gnueabi`, Debian 12
  binutils, GCC 12 for the host, cmake 3.27, ninja, qemu-arm-static) under `/tmp/lumatv-npu-tools`, fetches TensorFlow 2.13.1 and `stb_image.h`
  (pinned, checked), patches TFLite's CMake so that its NNAPI delegate builds on Linux (`patch_tflite.py`: the change webosose's
  `0007-enable-nnapi-delegate.patch` makes), builds TFLite with XNNPACK and NNAPI and its dependencies for each target, then `lumatv-npu-probe`,
  `libstub-npu-delegate.so`, `nnapi-stub/libneuralnetworks.so` and `nnapi-stub-12/libneuralnetworks.so` (stub NNAPI runtimes), `libnot-a-delegate.so`,
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
- `tests/run_tests.sh [host|arm]`: 50 checks against `stub/stub_delegate.cc` (LG's plugin surface: options, custom-op-only claim rule, refusal
  text, `use_custom_alloc` through the exported `SetCustomAllocationForTensor`, a configurable device delay) and `stub/stub_nnapi.cc` (a stand-in
  NNAPI runtime with the devices `lgnpu-stub`, an accelerator, and `nnapi-reference`, a CPU; it computes float32 ADD, so data crossing between CPU
  and device can be checked, and accepts every other operation it claims without computing it, as LG's HAL does; switches `STUB_NNAPI_OPS`,
  `_DEVICES`, `_DELAY_US`, `_FAIL=compile|compute`, `_LOG`; a second build leaves out the 1.3 feature-level call) and generated models, among
  them one with exactly LG's op set (`tests/make_models.cc`) and one with a lone `lgnpu_custom_op`, and the real detector on LG's sample
  pictures. The NNAPI checks cover device listing and selection, partial and full claims, a device that computes nothing (exit 5), failing
  compilation and execution, a missing runtime, and which of LG's op set TFLite hands an NNAPI 1.2 device (all but `QUANTIZE` and
  `RESIZE_NEAREST_NEIGHBOR`). Passing on both builds (the ARM one under qemu-arm-static).
- `tests/lg/run.sh` (needs the copied delegate): the 18 checks of the entry section, on LG's real binary.
- `tests/lg/hal_mock.sh` (needs the copied HAL files): the 19 checks of the section on the online NN HAL, on LG's real test binary and library.

## TV slot

Needs Main's slot; read only except for CPU load and files under `/tmp/lumatv-npu`. The set at Home, awake. About 20 minutes with 100 runs (the builtin-kernel runs dominate; `--runs 30` takes about 6).
Push (from the workstation, with the repo's `docs/research/arch/tv.py`):

```
python3 - <<'EOF'
import sys; sys.path.insert(0, "docs/research/arch")
import tv
tv.tv("mkdir -p /tmp/lumatv-npu/models")
tv.put("npuprobe/build/arm/lumatv-npu-probe", "/tmp/lumatv-npu/lumatv-npu-probe")
tv.put("npuprobe/tv-probe.sh", "/tmp/lumatv-npu/tv-probe.sh")
for f in ("efficientdet_lite0_320_ptq.tflite", "coco_labels.txt"):
    tv.put("/tmp/lumatv-npu-models/" + f, "/tmp/lumatv-npu/models/" + f)
tv.tv("chmod +x /tmp/lumatv-npu/lumatv-npu-probe")
print(tv.tv("sh /tmp/lumatv-npu/tv-probe.sh --runs 100 --threads 3 --no-luna", timeout=1800))
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
| 7 | state after, `dmesg` lines about `npu` | empty process list, no `npu` error lines | nothing left held |

`tv-probe.sh` writes `00-before.txt` to `07-after.txt` and `summary.txt` (one line per step) under `/tmp/lumatv-npu/results`.

Rollback: nothing outside `/tmp/lumatv-npu` changes; `rm -rf /tmp/lumatv-npu`. The probe holds `/dev/npu0` only inside steps 6 and 6b (in 6b only if the HAL is not the stand-in). The script
prints the `aipq compatibility` line at step 0 and again at step 6b: `false` means the NPU is not shared with AI picture (`ai_pq`), so if AI picture
is on and the picture changes during a run, stop the probe; switching AI picture back on is a person's action in the TV's settings. If a
run hangs, kill the probe (`kill -9`): the driver keeps one context per open file [INFERENCE: `npu_drv.c` per-open `inst_id`, `hardware-census.md`], so closing the
descriptor frees it; the status files read as in step 0. A reboot is infrastructure and Nitin's decision (Principle 0). The slot sets no LG
setting and calls no luna method outside the query group.

## How the camera pop-ups would call it

`ha-cameras-result.md`: Home Assistant's own detectors (Reolink, Scrypted, Kestrel) already send `person`, `vehicle`, `animal` events; the
helper's `cameras.rs` turns them into `cameras.popup`, and `GET /v1/cameras/{id}/frame?w=&h=` serves the camera's current picture. The TV's
detector would cover the cameras that report no kind, and confirm or drop a plain `motion` event. Shape, nothing of it written yet:

- **Worker.** `lumatv-detect`, a 32-bit process the helper starts with a connected socket as fd 0 (as `lumatv-jpeg`), holding the model
  resident (about 30 MB). One request in flight. Protocol like `jpeg/jpegproto.h`: request `op=DETECT`, `format` (RGB888 or BGRA), `w`, `h`,
  `min_score`, the pixels; reply `status`, `backend` (`cpu` or `npu`), `ms`, `n`, then `n` records `{class, score, ymin, xmin, ymax, xmax}` in 0..1
  of the picture. It `nice`s itself; the helper never has two requests outstanding and refuses one while a panel or an app asked for the frame
  budget.
- **Helper route.** `GET /v1/cameras/{id}/detect?minScore=` runs the worker on the picture the frame route already holds (`frameHoldMs`) and
  answers `{"camera","at","ms","backend","detections":[{"kind":"person","label":"person","score":0.91,"box":[y0,x0,y1,x1]}]}`. `kind` is the pop-up
  vocabulary: person (id 0), vehicle (1, 2, 3, 5, 7), animal (15 to 24); anything else is left out. The same result goes to the hub topic
  `cameras.detect` (latest per camera) and, when a kind is in `cameras.popupKinds` and the cooldown allows, becomes an event through the
  existing path (`POST /v1/cameras/event` with `source:"lumatv-detect"`), so the pop-up policy, cooldown and clearing stay as they are.
- **Config.** `cameras.detect.enabled` (false), `intervalSeconds` per camera without a detector (0: only on a motion event), `minScore` (0.5),
  `threads` (2), `backend` (`cpu`; `npu` only once a package exists). Off: no process, no model load, no fetch.
- **Budget.** At 80 to 120 ms a frame on 3 threads (about 0.3 core-seconds), a dozen cameras sampled every 10 s cost about 0.36 of a core, 12 % of
  the three at Home [INFERENCE from the expected speed]; run only while the screen is on or a pop-up is wanted.
