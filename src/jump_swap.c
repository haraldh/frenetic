void jump_swap(void *from[5], void *into[5]) {
    if(__builtin_setjmp(from) == 0) {
        __builtin_longjmp(into, 1);
    }
}
