// Freestanding runtime for the Zeta kernel: the handful of C-library symbols the
// native backend references — malloc/free/memcpy/memcmp/memset. This is what lets
// the kernel use the full Zeta type system (String/array/struct), which allocate
// and copy via these symbols.
//
// The allocator is a RECLAIMING first-fit free list over a static arena: blocks
// carry a header and form a single address-ordered list spanning the arena;
// malloc splits a large-enough free block, free marks it and coalesces adjacent
// free runs. So a long-running kernel that allocates and drops in a loop stays
// within the arena instead of leaking (unlike a bump pointer).
//
// Build with clang --target=riscv64 -ffreestanding -nostdlib -O0. -O0 matters:
// at higher opt levels LLVM's loop-idiom pass rewrites the byte-copy loops below
// into calls to memcpy/memset — i.e. into themselves — which would recurse.

typedef unsigned long size_t;

#define ARENA_SIZE (4u << 20) // 4 MiB
#define ALIGN 16u

// One header precedes each block's payload. `size` is the payload byte count;
// blocks are linked in ascending address order so coalescing is a list walk.
typedef struct Block {
    size_t size;
    struct Block *next;
    int free;
} Block;

static unsigned char arena[ARENA_SIZE] __attribute__((aligned(ALIGN)));
static Block *heap = 0;

static size_t align_up(size_t n) {
    return (n + (ALIGN - 1u)) & ~((size_t)(ALIGN - 1u));
}

static void heap_init(void) {
    heap = (Block *)arena;
    heap->size = ARENA_SIZE - sizeof(Block);
    heap->next = 0;
    heap->free = 1;
}

void *malloc(size_t n) {
    if (!heap) {
        heap_init();
    }
    n = align_up(n);
    for (Block *b = heap; b; b = b->next) {
        if (b->free && b->size >= n) {
            // Split off the remainder if it can hold a header + a min payload.
            if (b->size >= n + sizeof(Block) + ALIGN) {
                Block *nb = (Block *)((unsigned char *)(b + 1) + n);
                nb->size = b->size - n - sizeof(Block);
                nb->next = b->next;
                nb->free = 1;
                b->size = n;
                b->next = nb;
            }
            b->free = 0;
            return (void *)(b + 1);
        }
    }
    return 0; // out of arena → null
}

void free(void *p) {
    if (!p) {
        return;
    }
    Block *b = ((Block *)p) - 1;
    b->free = 1;
    // One left-to-right pass merges every run of adjacent free blocks.
    for (Block *c = heap; c; c = c->next) {
        while (c->free && c->next && c->next->free) {
            c->size += sizeof(Block) + c->next->size;
            c->next = c->next->next;
        }
    }
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
