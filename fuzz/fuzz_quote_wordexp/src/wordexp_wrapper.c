#include <wordexp.h>
#include <stddef.h>

static _Thread_local wordexp_t we;

const char *wordexp_wrapper(const char *words, char ***wordv_p, size_t *wordc_p) {
    int res = wordexp(words, &we, WRDE_NOCMD | WRDE_SHOWERR | WRDE_UNDEF);
    *wordv_p = we.we_wordv;
    *wordc_p = we.we_wordc;
    switch (res) {
        case 0: return NULL;
        case WRDE_BADCHAR: return "WRDE_BADCHAR";
        case WRDE_BADVAL: return "WRDE_BADVAL";
        case WRDE_CMDSUB: return "WRDE_CMDSUB";
        case WRDE_NOSPACE: return "WRDE_NOSPACE";
        case WRDE_SYNTAX: return "WRDE_SYNTAX";
        default: return "[unknown wordexp error]";
    }
}

void wordfree_wrapper() {
    wordfree(&we);
}
