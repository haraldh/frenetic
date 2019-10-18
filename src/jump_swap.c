extern void jump_stack(void *stack, void *coro, void *func);

void jump_init(void *from[5], void *stack, void *coro, void *func) {
    if(__builtin_setjmp(from) == 0) {
        jump_stack(stack, coro, func);
    }
}

void jump_swap(void *from[5], void *into[5]) {
    if(__builtin_setjmp(from) == 0) {
        __builtin_longjmp(into, 1);
    }
}

void jump_into(void *into[5]) {
    __builtin_longjmp(into, 1);
}