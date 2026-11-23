/* Stand-ins for the NEON types and intrinsics scale.c uses, for builds without NEON (the x86_64 host tests): the vector code
 * that runs on the set runs here too, lane for lane. Only the AArch64 forms scale.c calls are here; each comment names the
 * instruction the intrinsic is. Little-endian, like both targets. The vector types are the compilers' own (GNU vector
 * extensions), so ordinary arithmetic on them is lane-wise and wraps exactly as the unsigned NEON instructions do. */
#ifndef LUMATV_NEON_EMU_H
#define LUMATV_NEON_EMU_H

#ifdef __ARM_NEON
#error "neon_emu.h stands in for <arm_neon.h> on builds without NEON"
#endif

#include <stdint.h>
#include <string.h>

#define EMU_FN static inline __attribute__((always_inline))

typedef uint8_t uint8x8_t __attribute__((vector_size(8)));
typedef uint8_t uint8x16_t __attribute__((vector_size(16)));
typedef uint16_t uint16x4_t __attribute__((vector_size(8)));
typedef uint16_t uint16x8_t __attribute__((vector_size(16)));
typedef uint32_t uint32x4_t __attribute__((vector_size(16)));
typedef int16_t int16x4_t __attribute__((vector_size(8)));
typedef int16_t int16x8_t __attribute__((vector_size(16)));
typedef int32_t int32x4_t __attribute__((vector_size(16)));
typedef struct {
    uint8x16_t val[2];
} uint8x16x2_t;
typedef struct {
    uint8x16_t val[4];
} uint8x16x4_t;

/* LD1 / ST1: no alignment needed */
EMU_FN uint8x16_t vld1q_u8(const uint8_t *p)
{
    uint8x16_t r;
    memcpy(&r, p, sizeof(r));
    return r;
}
EMU_FN uint16x8_t vld1q_u16(const uint16_t *p)
{
    uint16x8_t r;
    memcpy(&r, p, sizeof(r));
    return r;
}
EMU_FN uint32x4_t vld1q_u32(const uint32_t *p)
{
    uint32x4_t r;
    memcpy(&r, p, sizeof(r));
    return r;
}
EMU_FN uint8x16x2_t vld1q_u8_x2(const uint8_t *p)
{
    uint8x16x2_t r;
    memcpy(&r, p, sizeof(r));
    return r;
}
EMU_FN uint8x16x4_t vld1q_u8_x4(const uint8_t *p)
{
    uint8x16x4_t r;
    memcpy(&r, p, sizeof(r));
    return r;
}
EMU_FN void vst1q_u8(uint8_t *p, uint8x16_t a)
{
    memcpy(p, &a, sizeof(a));
}
EMU_FN void vst1q_u32(uint32_t *p, uint32x4_t a)
{
    memcpy(p, &a, sizeof(a));
}

/* TBL with two or four table registers: lane i is table byte idx[i], or 0 when idx[i] is past the table */
EMU_FN uint8x16_t vqtbl2q_u8(uint8x16x2_t t, uint8x16_t idx)
{
    uint8_t tab[32];
    uint8x16_t r = { 0 };
    memcpy(tab, &t, sizeof(tab));
    for (int i = 0; i < 16; i++)
        r[i] = idx[i] < 32 ? tab[idx[i]] : 0;
    return r;
}
EMU_FN uint8x16_t vqtbl4q_u8(uint8x16x4_t t, uint8x16_t idx)
{
    uint8_t tab[64];
    uint8x16_t r = { 0 };
    memcpy(tab, &t, sizeof(tab));
    for (int i = 0; i < 16; i++)
        r[i] = idx[i] < 64 ? tab[idx[i]] : 0;
    return r;
}

/* halves and joins: no instruction of their own, or MOV */
EMU_FN uint8x8_t vget_low_u8(uint8x16_t a)
{
    return __builtin_shufflevector(a, a, 0, 1, 2, 3, 4, 5, 6, 7);
}
EMU_FN uint8x8_t vget_high_u8(uint8x16_t a)
{
    return __builtin_shufflevector(a, a, 8, 9, 10, 11, 12, 13, 14, 15);
}
EMU_FN uint16x4_t vget_low_u16(uint16x8_t a)
{
    return __builtin_shufflevector(a, a, 0, 1, 2, 3);
}
EMU_FN uint16x4_t vget_high_u16(uint16x8_t a)
{
    return __builtin_shufflevector(a, a, 4, 5, 6, 7);
}
EMU_FN int16x8_t vcombine_s16(int16x4_t lo, int16x4_t hi)
{
    return __builtin_shufflevector(lo, hi, 0, 1, 2, 3, 4, 5, 6, 7);
}
EMU_FN uint8x16_t vcombine_u8(uint8x8_t lo, uint8x8_t hi)
{
    return __builtin_shufflevector(lo, hi, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15);
}

/* UXTL, UXTL2: zero-extend */
EMU_FN uint16x8_t vmovl_u8(uint8x8_t a)
{
    return __builtin_convertvector(a, uint16x8_t);
}
EMU_FN uint16x8_t vmovl_high_u8(uint8x16_t a)
{
    return vmovl_u8(vget_high_u8(a));
}

/* UMLAL, UMLAL2: acc + a*b with the product taken at 32 bits */
EMU_FN uint32x4_t vmlal_u16(uint32x4_t acc, uint16x4_t a, uint16x4_t b)
{
    return acc + __builtin_convertvector(a, uint32x4_t) * __builtin_convertvector(b, uint32x4_t);
}
EMU_FN uint32x4_t vmlal_high_u16(uint32x4_t acc, uint16x8_t a, uint16x8_t b)
{
    return vmlal_u16(acc, vget_high_u16(a), vget_high_u16(b));
}

EMU_FN uint32x4_t vdupq_n_u32(uint32_t x)
{
    return (uint32x4_t){ x, x, x, x };
}
EMU_FN int32x4_t vdupq_n_s32(int32_t x)
{
    return (int32x4_t){ x, x, x, x };
}
EMU_FN int32x4_t vreinterpretq_s32_u32(uint32x4_t a)
{
    return (int32x4_t)a;
}

/* MUL, MLA by element: the low 32 bits of the product */
EMU_FN uint32x4_t vmulq_n_u32(uint32x4_t a, uint32_t s)
{
    return a * vdupq_n_u32(s);
}
EMU_FN uint32x4_t vmlaq_n_u32(uint32x4_t acc, uint32x4_t a, uint32_t s)
{
    return acc + a * vdupq_n_u32(s);
}

/* SQDMULH: the high 32 bits of 2*a*b, which is floor(a*b / 2^31) while a and b are not both INT32_MIN (that one saturates) */
EMU_FN int32x4_t vqdmulhq_s32(int32x4_t a, int32x4_t b)
{
    int32x4_t r = { 0 };
    for (int i = 0; i < 4; i++)
        r[i] = (a[i] == INT32_MIN && b[i] == INT32_MIN) ? INT32_MAX : (int32_t)(((int64_t)a[i] * b[i] * 2) >> 32);
    return r;
}

/* RSHRN: (a + 2^(n-1)) >> n taken on the unsigned 32-bit lane in wider arithmetic, then the low 16 bits */
EMU_FN int16x4_t vrshrn_n_s32(int32x4_t a, const int n)
{
    int16x4_t r = { 0 };
    for (int i = 0; i < 4; i++)
        r[i] = (int16_t)(((uint64_t)(uint32_t)a[i] + ((uint64_t)1 << (n - 1))) >> n);
    return r;
}

/* SQXTUN: signed lane to unsigned byte, saturating */
EMU_FN uint8x8_t vqmovun_s16(int16x8_t a)
{
    uint8x8_t r = { 0 };
    for (int i = 0; i < 8; i++)
        r[i] = a[i] < 0 ? 0 : a[i] > 255 ? 255 : (uint8_t)a[i];
    return r;
}

#endif
