The four headers are libjpeg-turbo 2.1.4's (from the webOS NDK sysroot), copied so `lumatv-jpeg` can name the types of
`libjpeg.so.62`, the library the set ships. One edit: `JPEG_LIB_VERSION` is 62 in `jconfig.h` (the NDK's is 80), which is the
struct layout of the set's library; `jpeg_CreateDecompress` refuses a mismatch, so a wrong layout fails loudly. No libjpeg code
is linked: the program opens the library at run time. The headers are under the IJG licence and libjpeg-turbo's BSD-3-Clause
licence (see the file heads).
