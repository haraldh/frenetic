declare void @llvm.stackrestore(i8*) nounwind

; This function sets up the coroutine. It does the following steps:
;   1. Call setjmp().
;   2. Set the stack pointer to %stackaddr.
;   3. Call %func(%c, %f).
define dso_local void
@jump_stack(i8* %stackaddr, i8* %c, void (i8*)* %func)
nounwind
{
  %tc = alloca i8*, align 4
  %tfunc = alloca void (i8*)*, align 4

  store i8* %c, i8** %tc
  store void (i8*)* %func, void (i8*)** %tfunc

  %gc = load volatile i8*, i8** %tc
  %gfunc = load volatile void (i8*)*, void (i8*)** %tfunc

  call void @llvm.stackrestore(i8* %stackaddr)   ; Move onto new stack %stackaddr

  call void %gfunc(i8* %gc) noreturn; Call %func(%buff, %c, %f)
  unreachable
}
