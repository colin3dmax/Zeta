// Freestanding runtime for the Zeta kernel: the handful of C-library symbols the
// native backend references — malloc/free/memcpy/memcmp/memset. This is what lets
// the kernel use the full Zeta type system (String/array/struct), which allocate
// and copy via these symbols.
//
// The allocator is a bump pointer over a static arena; free() is a no-op (the
// demo kernel allocates a bounded amount, so never reclaiming is fine). A real
// kernel would swap this for a buddy/slab allocator behind the same symbols.
//
// Build with clang --target=riscv64 -ffreestanding -nostdlib -O0. -O0 matters:
// at higher opt levels LLVM's loop-idiom pass rewrites the byte-copy loops below
// into calls to memcpy/memset — i.e. into themselves — which would recurse.

typedef unsigned long size_t;

#define ARENA_SIZE (4u << 20) // 4 MiB

static unsigned char arena[ARENA_SIZE];
static size_t bump = 0;

void *malloc(size_t n) {
    size_t off = (bump + 15u) & ~((size_t)15u); // 16-byte align
    if (off + n > ARENA_SIZE) {
        return 0; // out of arena → null
    }
    bump = off + n;
    return &arena[off];
}

void free(void *p) {
    (void)p; // bump allocator never reclaims
}

void *memcpy(void *dst, const void *src, size_t n) {
    unsigned char *d = (unsigned char *)dst;
    const unsigned char *s = (const unsigned char *)src;
    for (size_t i = 0; i < n; i++) {
        d[i] = s[i];
    }
    return dst;
}

void *memset(void *dst, int c, size_t n) {
    unsigned char *d = (unsigned char *)dst;
    for (size_t i = 0; i < n; i++) {
        d[i] = (unsigned char)c;
    }
    return dst;
}

int memcmp(const void *a, const void *b, size_t n) {
    const unsigned char *x = (const unsigned char *)a;
    const unsigned char *y = (const unsigned char *)b;
    for (size_t i = 0; i < n; i++) {
        if (x[i] != y[i]) {
            return (int)x[i] - (int)y[i];
        }
    }
    return 0;
}
