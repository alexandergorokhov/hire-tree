/* fdshim: lets neard (near-sandbox) start where the open-file hard limit
 * is below the 65535 it asks for and cannot be raised (some containers).
 * A request to raise RLIMIT_NOFILE above the hard limit is clamped to the
 * hard limit instead of failing. Nothing else changes. `make sandbox`
 * loads it with LD_PRELOAD only when `ulimit -n 65535` fails. The sandbox
 * node's store uses max_open_files = 3000, far below any such limit.
 * Linux x86-64 / glibc only. Headers are avoided on purpose so the
 * signatures below match the exported symbols exactly. */
#define _GNU_SOURCE
#include <dlfcn.h>

#define NOFILE 7 /* RLIMIT_NOFILE on Linux */

struct rl64 {
    unsigned long long cur;
    unsigned long long max;
};
typedef int (*prlimit_fn)(int, int, const struct rl64 *, struct rl64 *);

static prlimit_fn real_prlimit(void) {
    static prlimit_fn f;
    if (!f) f = (prlimit_fn)dlsym(RTLD_NEXT, "prlimit64");
    return f;
}

int prlimit64(int pid, int res, const struct rl64 *nw, struct rl64 *old) {
    prlimit_fn real = real_prlimit();
    if (nw && res == NOFILE) {
        struct rl64 cur, want = *nw;
        if (real(0, NOFILE, 0, &cur) == 0) {
            if (want.max > cur.max) want.max = cur.max;
            if (want.cur > want.max) want.cur = want.max;
        }
        return real(pid, res, &want, old);
    }
    return real(pid, res, nw, old);
}

int prlimit(int pid, int res, const struct rl64 *nw, struct rl64 *old) {
    return prlimit64(pid, res, nw, old);
}

int setrlimit64(int res, const struct rl64 *rl) { return prlimit64(0, res, rl, 0); }

int setrlimit(int res, const struct rl64 *rl) { return prlimit64(0, res, rl, 0); }
