// Read-only DRM/KMS inventory for the C4's lg1k card: CRTCs, connectors,
// encoders, planes, every object's properties, IN_FORMATS blobs and the
// current framebuffers. No modeset, no atomic commit, no page flip: only
// GET ioctls and two per-fd client caps. Build:
//   zig cc -target aarch64-linux-musl -static -O2 -o drmplanes drmplanes.c
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>
#include <errno.h>

#define DRM_IOCTL_BASE 'd'
#define DRM_IOWR(nr, type) _IOWR(DRM_IOCTL_BASE, nr, type)
#define DRM_IOW(nr, type) _IOW(DRM_IOCTL_BASE, nr, type)

struct drm_version { int major, minor, patchlevel; size_t name_len; char *name; size_t date_len; char *date; size_t desc_len; char *desc; };
struct drm_set_client_cap { uint64_t capability, value; };
struct drm_get_cap { uint64_t capability, value; };
struct drm_mode_card_res { uint64_t fb_id_ptr, crtc_id_ptr, connector_id_ptr, encoder_id_ptr; uint32_t count_fbs, count_crtcs, count_connectors, count_encoders, min_width, max_width, min_height, max_height; };
struct drm_mode_modeinfo { uint32_t clock; uint16_t hdisplay, hsync_start, hsync_end, htotal, hskew, vdisplay, vsync_start, vsync_end, vtotal, vscan; uint32_t vrefresh, flags, type; char name[32]; };
struct drm_mode_crtc { uint64_t set_connectors_ptr; uint32_t count_connectors, crtc_id, fb_id, x, y, gamma_size, mode_valid; struct drm_mode_modeinfo mode; };
struct drm_mode_get_connector { uint64_t encoders_ptr, modes_ptr, props_ptr, prop_values_ptr; uint32_t count_modes, count_props, count_encoders, encoder_id, connector_id, connector_type, connector_type_id, connection, mm_width, mm_height, subpixel, pad; };
struct drm_mode_get_encoder { uint32_t encoder_id, encoder_type, crtc_id, possible_crtcs, possible_clones; };
struct drm_mode_get_plane_res { uint64_t plane_id_ptr; uint32_t count_planes; };
struct drm_mode_get_plane { uint32_t plane_id, crtc_id, fb_id, possible_crtcs, gamma_size, count_format_types; uint64_t format_type_ptr; };
struct drm_mode_obj_get_properties { uint64_t props_ptr, prop_values_ptr; uint32_t count_props, obj_id, obj_type; };
struct drm_mode_property_enum { uint64_t value; char name[32]; };
struct drm_mode_get_property { uint64_t values_ptr, enum_blob_ptr; uint32_t prop_id, flags; char name[32]; uint32_t count_values, count_enum_blobs; };
struct drm_mode_get_blob { uint32_t blob_id, length; uint64_t data; };
struct drm_mode_fb_cmd { uint32_t fb_id, width, height, pitch, bpp, depth, handle; };
struct drm_gem_close { uint32_t handle, pad; };
struct drm_format_modifier_blob { uint32_t version, flags, count_formats, formats_offset, count_modifiers, modifiers_offset; };
struct drm_format_modifier { uint64_t formats; uint32_t offset, pad; uint64_t modifier; };

#define DRM_IOCTL_VERSION DRM_IOWR(0x00, struct drm_version)
#define DRM_IOCTL_GET_CAP DRM_IOWR(0x0c, struct drm_get_cap)
#define DRM_IOCTL_SET_CLIENT_CAP DRM_IOW(0x0d, struct drm_set_client_cap)
#define DRM_IOCTL_GEM_CLOSE DRM_IOW(0x09, struct drm_gem_close)
#define DRM_IOCTL_MODE_GETRESOURCES DRM_IOWR(0xA0, struct drm_mode_card_res)
#define DRM_IOCTL_MODE_GETCRTC DRM_IOWR(0xA1, struct drm_mode_crtc)
#define DRM_IOCTL_MODE_GETENCODER DRM_IOWR(0xA6, struct drm_mode_get_encoder)
#define DRM_IOCTL_MODE_GETCONNECTOR DRM_IOWR(0xA7, struct drm_mode_get_connector)
#define DRM_IOCTL_MODE_GETPROPERTY DRM_IOWR(0xAA, struct drm_mode_get_property)
#define DRM_IOCTL_MODE_GETPROPBLOB DRM_IOWR(0xAC, struct drm_mode_get_blob)
#define DRM_IOCTL_MODE_GETFB DRM_IOWR(0xAD, struct drm_mode_fb_cmd)
#define DRM_IOCTL_MODE_GETPLANERESOURCES DRM_IOWR(0xB5, struct drm_mode_get_plane_res)
#define DRM_IOCTL_MODE_GETPLANE DRM_IOWR(0xB6, struct drm_mode_get_plane)
#define DRM_IOCTL_MODE_OBJ_GETPROPERTIES DRM_IOWR(0xB9, struct drm_mode_obj_get_properties)

#define DRM_MODE_OBJECT_CRTC 0xcccccccc
#define DRM_MODE_OBJECT_CONNECTOR 0xc0c0c0c0
#define DRM_MODE_OBJECT_PLANE 0xeeeeeeee
#define DRM_MODE_PROP_PENDING (1<<0)
#define DRM_MODE_PROP_RANGE (1<<1)
#define DRM_MODE_PROP_IMMUTABLE (1<<2)
#define DRM_MODE_PROP_ENUM (1<<3)
#define DRM_MODE_PROP_BLOB (1<<4)
#define DRM_MODE_PROP_BITMASK (1<<5)
#define DRM_MODE_PROP_EXTENDED_TYPE 0x0000ffc0
#define DRM_MODE_PROP_OBJECT (1<<6)
#define DRM_MODE_PROP_SIGNED_RANGE (2<<6)

static int fd;
static void fourcc(uint32_t f, char *s) { s[0]=f&255; s[1]=(f>>8)&255; s[2]=(f>>16)&255; s[3]=(f>>24)&255; s[4]=0; for (int i=0;i<4;i++) if (s[i]<32||s[i]>126) s[i]='?'; }

static void dump_blob(uint32_t id, const char *propname) {
  struct drm_mode_get_blob b = { id, 0, 0 };
  if (ioctl(fd, DRM_IOCTL_MODE_GETPROPBLOB, &b) < 0) { printf("      blob %u: %s\n", id, strerror(errno)); return; }
  if (!b.length) { printf("      blob %u: empty\n", id); return; }
  uint8_t *d = malloc(b.length); b.data = (uintptr_t)d;
  if (ioctl(fd, DRM_IOCTL_MODE_GETPROPBLOB, &b) < 0) { printf("      blob %u: %s\n", id, strerror(errno)); free(d); return; }
  printf("      blob %u: %u bytes", id, b.length);
  if (!strcmp(propname, "IN_FORMATS") && b.length >= sizeof(struct drm_format_modifier_blob)) {
    struct drm_format_modifier_blob *h = (void *)d;
    uint32_t *fm = (void *)(d + h->formats_offset);
    struct drm_format_modifier *mods = (void *)(d + h->modifiers_offset);
    printf(" (v%u, %u formats, %u modifiers)\n", h->version, h->count_formats, h->count_modifiers);
    for (uint32_t m = 0; m < h->count_modifiers; m++) {
      printf("        modifier 0x%016llx:", (unsigned long long)mods[m].modifier);
      for (uint32_t i = 0; i < 64 && mods[m].offset + i < h->count_formats; i++)
        if (mods[m].formats & (1ull << i)) { char s[5]; fourcc(fm[mods[m].offset + i], s); printf(" %s", s); }
      printf("\n");
    }
  } else if (!strcmp(propname, "MODE_ID") && b.length == sizeof(struct drm_mode_modeinfo)) {
    struct drm_mode_modeinfo *m = (void *)d;
    printf(" mode %s %ux%u@%u clock %u flags 0x%x\n", m->name, m->hdisplay, m->vdisplay, m->vrefresh, m->clock, m->flags);
  } else {
    printf(":");
    for (uint32_t i = 0; i < b.length && i < 64; i++) printf(" %02x", d[i]);
    printf("%s\n", b.length > 64 ? " ..." : "");
  }
  free(d);
}

static void dump_props(uint32_t obj, uint32_t type) {
  struct drm_mode_obj_get_properties p = { 0, 0, 0, obj, type };
  if (ioctl(fd, DRM_IOCTL_MODE_OBJ_GETPROPERTIES, &p) < 0) { printf("    props: %s\n", strerror(errno)); return; }
  uint32_t n = p.count_props; if (!n) { printf("    props: none\n"); return; }
  uint32_t *ids = calloc(n, 4); uint64_t *vals = calloc(n, 8);
  p.props_ptr = (uintptr_t)ids; p.prop_values_ptr = (uintptr_t)vals; p.count_props = n;
  if (ioctl(fd, DRM_IOCTL_MODE_OBJ_GETPROPERTIES, &p) < 0) { printf("    props: %s\n", strerror(errno)); return; }
  for (uint32_t i = 0; i < n; i++) {
    struct drm_mode_get_property gp = { 0, 0, ids[i], 0, "", 0, 0 };
    if (ioctl(fd, DRM_IOCTL_MODE_GETPROPERTY, &gp) < 0) { printf("    prop %u: %s\n", ids[i], strerror(errno)); continue; }
    uint64_t *values = NULL; struct drm_mode_property_enum *enums = NULL;
    if (gp.count_values) { values = calloc(gp.count_values, 8); gp.values_ptr = (uintptr_t)values; }
    if (gp.count_enum_blobs && (gp.flags & (DRM_MODE_PROP_ENUM | DRM_MODE_PROP_BITMASK))) { enums = calloc(gp.count_enum_blobs, sizeof *enums); gp.enum_blob_ptr = (uintptr_t)enums; }
    ioctl(fd, DRM_IOCTL_MODE_GETPROPERTY, &gp);
    uint32_t ext = gp.flags & DRM_MODE_PROP_EXTENDED_TYPE;
    const char *kind = gp.flags & DRM_MODE_PROP_RANGE ? "range" : gp.flags & DRM_MODE_PROP_ENUM ? "enum" : gp.flags & DRM_MODE_PROP_BLOB ? "blob" : gp.flags & DRM_MODE_PROP_BITMASK ? "bitmask" : ext == DRM_MODE_PROP_OBJECT ? "object" : ext == DRM_MODE_PROP_SIGNED_RANGE ? "srange" : "?";
    printf("    %-24s id %-3u %-7s%s = ", gp.name, ids[i], kind, gp.flags & DRM_MODE_PROP_IMMUTABLE ? " (immutable)" : "");
    if (gp.flags & DRM_MODE_PROP_RANGE) printf("%llu [%llu..%llu]", (unsigned long long)vals[i], (unsigned long long)values[0], (unsigned long long)values[1]);
    else if (ext == DRM_MODE_PROP_SIGNED_RANGE) printf("%lld [%lld..%lld]", (long long)vals[i], (long long)values[0], (long long)values[1]);
    else if (gp.flags & DRM_MODE_PROP_ENUM) { printf("%llu", (unsigned long long)vals[i]); for (uint32_t e = 0; e < gp.count_enum_blobs; e++) printf("%s%s%s", e ? "" : " {", enums[e].value == vals[i] ? "*" : "", enums[e].name), e + 1 < gp.count_enum_blobs ? printf(",") : printf("}"); }
    else if (gp.flags & DRM_MODE_PROP_BITMASK) { printf("0x%llx", (unsigned long long)vals[i]); for (uint32_t e = 0; e < gp.count_enum_blobs; e++) printf("%s%s(bit %llu)", e ? "," : " {", enums[e].name, (unsigned long long)enums[e].value); printf("}"); }
    else printf("%llu", (unsigned long long)vals[i]);
    printf("\n");
    if ((gp.flags & DRM_MODE_PROP_BLOB) && vals[i]) dump_blob(vals[i], gp.name);
    free(values); free(enums);
  }
  free(ids); free(vals);
}

int main(int argc, char **argv) {
  const char *node = argc > 1 ? argv[1] : "/dev/dri/card0";
  fd = open(node, O_RDWR | O_CLOEXEC);
  if (fd < 0) { perror(node); return 1; }
  char name[64] = "", date[64] = "", desc[128] = "";
  struct drm_version v = { 0, 0, 0, sizeof name - 1, name, sizeof date - 1, date, sizeof desc - 1, desc };
  ioctl(fd, DRM_IOCTL_VERSION, &v);
  printf("driver %s %d.%d.%d (%s) %s\n", name, v.major, v.minor, v.patchlevel, date, desc);
  static const struct { uint64_t cap; const char *n; } caps[] = { {1,"DUMB_BUFFER"}, {2,"VBLANK_HIGH_CRTC"}, {3,"DUMB_PREFERRED_DEPTH"}, {4,"DUMB_PREFER_SHADOW"}, {5,"PRIME"}, {6,"TIMESTAMP_MONOTONIC"}, {7,"ASYNC_PAGE_FLIP"}, {8,"CURSOR_WIDTH"}, {9,"CURSOR_HEIGHT"}, {0x10,"ADDFB2_MODIFIERS"}, {0x11,"PAGE_FLIP_TARGET"}, {0x12,"CRTC_IN_VBLANK_EVENT"}, {0x13,"SYNCOBJ"}, {0x14,"SYNCOBJ_TIMELINE"} };
  for (unsigned i = 0; i < sizeof caps / sizeof *caps; i++) { struct drm_get_cap c = { caps[i].cap, 0 }; if (!ioctl(fd, DRM_IOCTL_GET_CAP, &c)) printf("cap %s = %llu\n", caps[i].n, (unsigned long long)c.value); else printf("cap %s: %s\n", caps[i].n, strerror(errno)); }
  struct drm_set_client_cap cc = { 2, 1 }; printf("client cap UNIVERSAL_PLANES: %s\n", ioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, &cc) ? strerror(errno) : "ok");
  cc.capability = 3; printf("client cap ATOMIC: %s\n", ioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, &cc) ? strerror(errno) : "ok");
  cc.capability = 4; printf("client cap ASPECT_RATIO: %s\n", ioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, &cc) ? strerror(errno) : "ok");
  cc.capability = 5; printf("client cap WRITEBACK_CONNECTORS: %s\n", ioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, &cc) ? strerror(errno) : "ok");

  struct drm_mode_card_res r = {0};
  if (ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &r) < 0) { perror("GETRESOURCES"); return 1; }
  uint32_t *fbs = calloc(r.count_fbs + 1, 4), *crtcs = calloc(r.count_crtcs + 1, 4), *conns = calloc(r.count_connectors + 1, 4), *encs = calloc(r.count_encoders + 1, 4);
  r.fb_id_ptr = (uintptr_t)fbs; r.crtc_id_ptr = (uintptr_t)crtcs; r.connector_id_ptr = (uintptr_t)conns; r.encoder_id_ptr = (uintptr_t)encs;
  ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &r);
  printf("\nresources: %u fbs, %u crtcs, %u connectors, %u encoders; size %ux%u..%ux%u\n", r.count_fbs, r.count_crtcs, r.count_connectors, r.count_encoders, r.min_width, r.min_height, r.max_width, r.max_height);

  for (uint32_t i = 0; i < r.count_crtcs; i++) {
    struct drm_mode_crtc c = {0}; c.crtc_id = crtcs[i];
    if (ioctl(fd, DRM_IOCTL_MODE_GETCRTC, &c) < 0) { printf("\ncrtc %u: %s\n", crtcs[i], strerror(errno)); continue; }
    printf("\ncrtc %u (index %u): fb %u at %u,%u gamma %u mode %s %ux%u@%u (valid %u)\n", c.crtc_id, i, c.fb_id, c.x, c.y, c.gamma_size, c.mode.name, c.mode.hdisplay, c.mode.vdisplay, c.mode.vrefresh, c.mode_valid);
    dump_props(crtcs[i], DRM_MODE_OBJECT_CRTC);
  }
  for (uint32_t i = 0; i < r.count_encoders; i++) {
    struct drm_mode_get_encoder e = { encs[i], 0, 0, 0, 0 };
    if (!ioctl(fd, DRM_IOCTL_MODE_GETENCODER, &e)) printf("\nencoder %u: type %u crtc %u possible_crtcs 0x%x clones 0x%x\n", e.encoder_id, e.encoder_type, e.crtc_id, e.possible_crtcs, e.possible_clones);
  }
  for (uint32_t i = 0; i < r.count_connectors; i++) {
    struct drm_mode_get_connector c = {0}; c.connector_id = conns[i];
    if (ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &c) < 0) { printf("\nconnector %u: %s\n", conns[i], strerror(errno)); continue; }
    struct drm_mode_modeinfo *modes = calloc(c.count_modes + 1, sizeof *modes); uint32_t *ce = calloc(c.count_encoders + 1, 4);
    c.modes_ptr = (uintptr_t)modes; c.encoders_ptr = (uintptr_t)ce; c.count_props = 0;
    ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &c);
    printf("\nconnector %u: type %u-%u connection %u (1=connected) encoder %u %ux%u mm, %u modes, encoders:", c.connector_id, c.connector_type, c.connector_type_id, c.connection, c.encoder_id, c.mm_width, c.mm_height, c.count_modes);
    for (uint32_t k = 0; k < c.count_encoders; k++) printf(" %u", ce[k]);
    printf("\n");
    for (uint32_t m = 0; m < c.count_modes; m++) printf("    mode %-16s %ux%u@%u clock %u htotal %u vtotal %u flags 0x%x type 0x%x\n", modes[m].name, modes[m].hdisplay, modes[m].vdisplay, modes[m].vrefresh, modes[m].clock, modes[m].htotal, modes[m].vtotal, modes[m].flags, modes[m].type);
    dump_props(conns[i], DRM_MODE_OBJECT_CONNECTOR);
    free(modes); free(ce);
  }

  struct drm_mode_get_plane_res pr = {0};
  if (ioctl(fd, DRM_IOCTL_MODE_GETPLANERESOURCES, &pr) < 0) { perror("GETPLANERESOURCES"); return 1; }
  uint32_t *planes = calloc(pr.count_planes + 1, 4); pr.plane_id_ptr = (uintptr_t)planes;
  ioctl(fd, DRM_IOCTL_MODE_GETPLANERESOURCES, &pr);
  printf("\nplanes: %u\n", pr.count_planes);
  for (uint32_t i = 0; i < pr.count_planes; i++) {
    struct drm_mode_get_plane p = {0}; p.plane_id = planes[i];
    if (ioctl(fd, DRM_IOCTL_MODE_GETPLANE, &p) < 0) { printf("\nplane %u: %s\n", planes[i], strerror(errno)); continue; }
    uint32_t *fmts = calloc(p.count_format_types + 1, 4); p.format_type_ptr = (uintptr_t)fmts;
    ioctl(fd, DRM_IOCTL_MODE_GETPLANE, &p);
    printf("\nplane %u: crtc %u fb %u possible_crtcs 0x%x gamma %u formats (%u):", p.plane_id, p.crtc_id, p.fb_id, p.possible_crtcs, p.gamma_size, p.count_format_types);
    for (uint32_t k = 0; k < p.count_format_types; k++) { char s[5]; fourcc(fmts[k], s); printf(" %s", s); }
    printf("\n");
    dump_props(planes[i], DRM_MODE_OBJECT_PLANE);
    if (p.fb_id) {
      struct drm_mode_fb_cmd f = { p.fb_id, 0, 0, 0, 0, 0, 0 };
      if (!ioctl(fd, DRM_IOCTL_MODE_GETFB, &f)) { printf("    fb %u: %ux%u pitch %u bpp %u depth %u\n", f.fb_id, f.width, f.height, f.pitch, f.bpp, f.depth); if (f.handle) { struct drm_gem_close g = { f.handle, 0 }; ioctl(fd, DRM_IOCTL_GEM_CLOSE, &g); } }
      else printf("    fb %u: %s\n", p.fb_id, strerror(errno));
    }
    free(fmts);
  }
  printf("\nframebuffers owned by this fd: %u (expected 0)\n", r.count_fbs);
  close(fd);
  return 0;
}
