declare void @llvm.eh.sjlj.longjmp(i8*) noreturn nounwind
declare i32 @llvm.eh.sjlj.setjmp(i8*) nounwind
declare void @llvm.stackrestore(i8*) nounwind
declare i8* @llvm.frameaddress(i32) nounwind readnone
declare i8* @llvm.stacksave() nounwind
declare void @llvm.lifetime.start(i64, i8* nocapture) nounwind
declare void @llvm.lifetime.end(i64, i8* nocapture) nounwind

; This function is essentially what __builtin_longjmp() emits in C.
; The purpose is to expose this intrinsic to Rust (without requiring nightly).
define dso_local void
@jump_into([5 x i8*]* nonnull %into)
noreturn nounwind naked
{
  %buff = bitcast [5 x i8*]* %into to i8*
  call void @llvm.eh.sjlj.longjmp(i8* %buff) ; Call longjmp()
  unreachable
}

; This function performs a bidirectional context switch.
; It simply calls setjmp(%from) and then longjmp(%into).
define dso_local void
@jump_swap([5 x i8*]* nonnull %ctx, [5 x i8*]* nonnull %into)
nounwind
{
  %tinto = alloca [5 x i8*]*, align 4
  store [5 x i8*]* %into, [5 x i8*]** %tinto

  ; setjmp(%from)
  ; Store the frame address.
  %frame = call i8* @llvm.frameaddress(i32 0)
  %foff = getelementptr inbounds [5 x i8*], [5 x i8*]* %ctx, i64 0, i32 0
  store i8* %frame, i8** %foff

  ; Store the stack address.
  %stack = call i8* @llvm.stacksave()
  %soff = getelementptr inbounds [5 x i8*], [5 x i8*]* %ctx, i64 0, i32 2
  store i8* %stack, i8** %soff

  ; The rest are architecture specific and stored by setjmp().
  %buff = bitcast [5 x i8*]* %ctx to i8*
  %retv = call i32 @llvm.eh.sjlj.setjmp(i8* %buff) returns_twice

  %zero = icmp eq i32 %retv, 0
  br i1 %zero, label %jump, label %done

jump:                    ; setjmp(%from) returned 0
  %ginto = load volatile [5 x i8*]*, [5 x i8*]** %tinto
  %iinto = bitcast [5 x i8*]* %ginto to i8*

  call void @llvm.eh.sjlj.longjmp(i8* %iinto) noreturn; longjmp(%into)
  unreachable

done:                    ; setjmp(%from) returned !0
  ret void
}

; This function sets up the coroutine. It does the following steps:
;   1. Call setjmp().
;   2. Set the stack pointer to %stackaddr.
;   3. Call %func(%c, %f).
define dso_local void
@jump_init([5 x i8*]* %ctx, i8* %stackaddr, i8* %c, void (i8*)* %func)
nounwind
{
  %tc = alloca i8*, align 4
  %tfunc = alloca void (i8*)*, align 4

  store i8* %c, i8** %tc
  store void (i8*)* %func, void (i8*)** %tfunc

  ; Call setjmp(%buff)
  ; Store the frame address.
  %frame = call i8* @llvm.frameaddress(i32 0)
  %foff = getelementptr inbounds [5 x i8*], [5 x i8*]* %ctx, i64 0, i32 0
  store i8* %frame, i8** %foff

  ; Store the stack address.
  %stack = call i8* @llvm.stacksave()
  %soff = getelementptr inbounds [5 x i8*], [5 x i8*]* %ctx, i64 0, i32 2
  store i8* %stack, i8** %soff

  ; The rest are architecture specific and stored by setjmp().
  %buff = bitcast [5 x i8*]* %ctx to i8*
  %retv = call i32 @llvm.eh.sjlj.setjmp(i8* %buff) returns_twice

  %zero = icmp eq i32 %retv, 0
  br i1 %zero, label %next, label %done

next:                     ; setjmp(%buff) returned 0

  %gc = load volatile i8*, i8** %tc
  %gfunc = load volatile void (i8*)*, void (i8*)** %tfunc

  call void @llvm.stackrestore(i8* %stackaddr)   ; Move onto new stack %stackaddr
  call void %gfunc(i8* %gc) noreturn; Call %func(%buff, %c, %f)
  unreachable

done:                     ; setjmp(%buff) returned !0
  ret void
}

define dso_local zeroext i1 @stk_grows_up(i8*) noinline nounwind {
  %2 = alloca i8*, align 8
  store i8* %0, i8** %2, align 8
  %3 = load i8*, i8** %2, align 8
  %4 = bitcast i8** %2 to i8*
  %5 = icmp ult i8* %3, %4
  ret i1 %5
}