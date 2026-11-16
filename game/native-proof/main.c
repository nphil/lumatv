#include <SDL.h>
#include <GLES3/gl3.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <time.h>

#define MAX_FRAMES 3600
#define MAX_NS UINT64_C(15000000000)

struct sample { uint64_t frame, swap_start, swap_end; };
static struct sample samples[MAX_FRAMES];

static uint64_t now_ns(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t) != 0) return 0;
    return (uint64_t)t.tv_sec * UINT64_C(1000000000) + (uint64_t)t.tv_nsec;
}

#define GL_FUNCTIONS(X) \
    X(PFNGLGETSTRINGPROC, GetString) X(PFNGLVIEWPORTPROC, Viewport) \
    X(PFNGLCLEARCOLORPROC, ClearColor) X(PFNGLCLEARPROC, Clear) \
    X(PFNGLCREATESHADERPROC, CreateShader) X(PFNGLSHADERSOURCEPROC, ShaderSource) \
    X(PFNGLCOMPILESHADERPROC, CompileShader) X(PFNGLGETSHADERIVPROC, GetShaderiv) \
    X(PFNGLGETSHADERINFOLOGPROC, GetShaderInfoLog) X(PFNGLDELETESHADERPROC, DeleteShader) \
    X(PFNGLCREATEPROGRAMPROC, CreateProgram) X(PFNGLATTACHSHADERPROC, AttachShader) \
    X(PFNGLLINKPROGRAMPROC, LinkProgram) X(PFNGLGETPROGRAMIVPROC, GetProgramiv) \
    X(PFNGLGETPROGRAMINFOLOGPROC, GetProgramInfoLog) X(PFNGLDELETEPROGRAMPROC, DeleteProgram) \
    X(PFNGLUSEPROGRAMPROC, UseProgram) X(PFNGLGETUNIFORMLOCATIONPROC, GetUniformLocation) \
    X(PFNGLUNIFORM1FPROC, Uniform1f) X(PFNGLDRAWARRAYSPROC, DrawArrays) \
    X(PFNGLGENVERTEXARRAYSPROC, GenVertexArrays) X(PFNGLBINDVERTEXARRAYPROC, BindVertexArray) \
    X(PFNGLDELETEVERTEXARRAYSPROC, DeleteVertexArrays)
#define DECL(type, name) static type p##name;
GL_FUNCTIONS(DECL)
#undef DECL

static int load_gl(FILE *log) {
#define LOAD(type, name) do { p##name = (type)SDL_GL_GetProcAddress("gl" #name); if (!p##name) { fprintf(log, "missing gl%s\n", #name); return 0; } } while (0);
    GL_FUNCTIONS(LOAD)
#undef LOAD
    return 1;
}

static GLuint shader(FILE *log, GLenum kind, const char *source) {
    GLuint s = pCreateShader(kind);
    GLint ok = 0;
    if (!s) { fprintf(log, "shader creation failed\n"); return 0; }
    pShaderSource(s, 1, &source, NULL);
    pCompileShader(s);
    pGetShaderiv(s, GL_COMPILE_STATUS, &ok);
    if (!ok) {
        char message[1024] = {0};
        pGetShaderInfoLog(s, sizeof(message), NULL, message);
        fprintf(log, "shader compilation failed: %s\n", message);
        pDeleteShader(s);
        return 0;
    }
    return s;
}

int main(int argc, char **argv) {
    (void)argc; (void)argv;
    const char *home = getenv("HOME");
    char path[4096];
    if (!home || snprintf(path, sizeof(path), "%s/data", home) >= (int)sizeof(path)) {
        fprintf(stderr, "HOME missing or too long\n"); return 1;
    }
    if (mkdir(path, 0700) != 0) { struct stat st; if (stat(path, &st) != 0 || !S_ISDIR(st.st_mode)) { perror("data directory"); return 1; } }
    if (snprintf(path, sizeof(path), "%s/data/native-proof-timing.csv", home) >= (int)sizeof(path)) return 1;
    FILE *log = fopen(path, "w");
    if (!log) { perror("timing log"); return 1; }
    fprintf(log, "# Monotonic nanoseconds; swap completion cadence is not panel scanout.\n");
    fflush(log);
    SDL_Window *window = NULL;
    SDL_GLContext context = NULL;
    GLuint vertex = 0, fragment = 0, program = 0, vao = 0;
    int result = 1, initialized = 0;
    unsigned count = 0;
    SDL_SetHint(SDL_HINT_WEBOS_ACCESS_POLICY_KEYS_BACK, "true");
    SDL_SetHint(SDL_HINT_WEBOS_ACCESS_POLICY_KEYS_EXIT, "true");
    if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_EVENTS) != 0) { fprintf(log, "# SDL_Init: %s\n", SDL_GetError()); goto done; }
    initialized = 1;
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_PROFILE_MASK, SDL_GL_CONTEXT_PROFILE_ES);
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_MAJOR_VERSION, 3);
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_MINOR_VERSION, 0);
    SDL_GL_SetAttribute(SDL_GL_DOUBLEBUFFER, 1);
    window = SDL_CreateWindow("LumaGame native proof", SDL_WINDOWPOS_UNDEFINED, SDL_WINDOWPOS_UNDEFINED, 1280, 720, SDL_WINDOW_OPENGL | SDL_WINDOW_FULLSCREEN_DESKTOP);
    if (!window) { fprintf(log, "# SDL_CreateWindow: %s\n", SDL_GetError()); goto done; }
    context = SDL_GL_CreateContext(window);
    if (!context) { fprintf(log, "# SDL_GL_CreateContext: %s\n", SDL_GetError()); goto done; }
    if (!load_gl(log)) goto done;
    const char *version = (const char *)pGetString(GL_VERSION);
    const char *renderer = (const char *)pGetString(GL_RENDERER);
    fprintf(log, "# GL_VERSION=%s\n# GL_RENDERER=%s\n", version ? version : "unavailable", renderer ? renderer : "unavailable");
    if (!version || SDL_strncmp(version, "OpenGL ES 3", 11) != 0) { fprintf(log, "# GLES 3 required\n"); goto done; }
    int vsync = SDL_GL_SetSwapInterval(1);
    fprintf(log, "# swap_interval_request=1 result=%d actual=%d%s%s\n", vsync, SDL_GL_GetSwapInterval(), vsync ? " error=" : "", vsync ? SDL_GetError() : "");
    const char *vs = "#version 300 es\nprecision mediump float; uniform float t; const vec2 p[3]=vec2[3](vec2(0.0,0.5),vec2(-0.45,-0.35),vec2(0.45,-0.35)); void main(){float a=t; mat2 r=mat2(cos(a),-sin(a),sin(a),cos(a)); gl_Position=vec4(r*p[gl_VertexID],0.0,1.0);}";
    const char *fs = "#version 300 es\nprecision mediump float; out vec4 color; void main(){color=vec4(0.25,0.8,0.9,1.0);}";
    vertex = shader(log, GL_VERTEX_SHADER, vs); fragment = shader(log, GL_FRAGMENT_SHADER, fs);
    if (!vertex || !fragment) goto done;
    program = pCreateProgram();
    if (!program) { fprintf(log, "# program creation failed\n"); goto done; }
    pAttachShader(program, vertex); pAttachShader(program, fragment); pLinkProgram(program);
    GLint linked = 0; pGetProgramiv(program, GL_LINK_STATUS, &linked);
    if (!linked) { char message[1024] = {0}; pGetProgramInfoLog(program, sizeof(message), NULL, message); fprintf(log, "# program link failed: %s\n", message); goto done; }
    pGenVertexArrays(1, &vao); pBindVertexArray(vao); pUseProgram(program);
    GLint t = pGetUniformLocation(program, "t");
    int width, height; SDL_GL_GetDrawableSize(window, &width, &height); pViewport(0, 0, width, height);
    fprintf(log, "# drawable=%dx%d max_seconds=15 max_frames=%d\n", width, height, MAX_FRAMES); fflush(log);
    uint64_t start = now_ns();
    if (!start) { fprintf(log, "# monotonic clock unavailable\n"); goto done; }
    int running = 1, fallback = vsync != 0 || SDL_GL_GetSwapInterval() != 1;
    unsigned fast_frames = 0;
    if (fallback) fprintf(log, "# pacing_fallback=swap_interval_unavailable delay_ms=8\n");
    while (running && count < MAX_FRAMES) {
        uint64_t frame = now_ns();
        if (!frame || frame < start) { fprintf(log, "# monotonic clock failed\n"); goto done; }
        if (frame - start >= MAX_NS) break;
        SDL_Event e;
        while (SDL_PollEvent(&e)) {
            if (e.type == SDL_QUIT || (e.type == SDL_KEYDOWN && !e.key.repeat && (e.key.keysym.scancode == SDL_SCANCODE_ESCAPE || e.key.keysym.scancode == SDL_SCANCODE_AC_BACK || e.key.keysym.scancode == SDL_SCANCODE_WEBOS_BACK || e.key.keysym.scancode == SDL_SCANCODE_WEBOS_EXIT))) running = 0;
        }
        if (!running) break;
        pClearColor(0.025f, 0.04f, 0.09f, 1.0f); pClear(GL_COLOR_BUFFER_BIT);
        pUniform1f(t, (float)((double)(frame - start) / 1000000000.0)); pDrawArrays(GL_TRIANGLES, 0, 3);
        uint64_t before = now_ns(); SDL_GL_SwapWindow(window); uint64_t after = now_ns();
        if (!before || !after || before < frame || after < before) { fprintf(log, "# monotonic clock failed\n"); goto done; }
        samples[count++] = (struct sample){frame, before, after};
        if (!fallback) {
            fast_frames = after - frame < UINT64_C(2000000) ? fast_frames + 1 : 0;
            if (fast_frames == 8) {
                fallback = 1;
                fprintf(log, "# pacing_fallback=unpaced_swaps delay_ms=8 after_frame=%u\n", count);
            }
        }
        if (fallback) SDL_Delay(8);
    }
    result = 0;
done:
    if (count) {
        fprintf(log, "frame_start_ns,swap_start_ns,swap_end_ns\n");
        for (unsigned i = 0; i < count; ++i) fprintf(log, "%" PRIu64 ",%" PRIu64 ",%" PRIu64 "\n", samples[i].frame, samples[i].swap_start, samples[i].swap_end);
        uint64_t duration = samples[count - 1].swap_end - samples[0].swap_end;
        fprintf(log, "# summary frames=%u swap_completion_elapsed_ns=%" PRIu64 " swap_completion_fps=%.3f\n", count, duration, duration ? (count - 1) * 1000000000.0 / (double)duration : 0.0);
    }
    fprintf(log, "# exit_status=%d\n", result);
    if (program) pDeleteProgram(program);
    if (vertex) pDeleteShader(vertex);
    if (fragment) pDeleteShader(fragment);
    if (vao) pDeleteVertexArrays(1, &vao);
    if (context) SDL_GL_DeleteContext(context);
    if (window) SDL_DestroyWindow(window);
    if (initialized) SDL_Quit();
    if (fclose(log) != 0) return 1;
    return result;
}
