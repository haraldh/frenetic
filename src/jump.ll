declare void @llvm.eh.sjlj.longjmp(i8*) noreturn nounwind
declare i32 @llvm.eh.sjlj.setjmp(i8*) nounwind
declare void @llvm.stackrestore(i8*) nounwind
declare i8* @llvm.frameaddress(i32) nounwind readnone
declare i8* @llvm.stacksave() nounwind
declare void @llvm.lifetime.start(i64, i8* nocapture) nounwind

; We put it here and mark it as alwaysinline for code-reuse.
; This function is internal only.
define private i32
@jump_save([5 x i8*]* nonnull %ctx)
alwaysinline nounwind naked returns_twice
{
  ; Store the frame address.
  %frame = tail call i8* @llvm.frameaddress(i32 0)
  %foff = getelementptr inbounds [5 x i8*], [5 x i8*]* %ctx, i64 0, i64 0
  store i8* %frame, i8** %foff, align 16

  ; Store the stack address.
  %stack = tail call i8* @llvm.stacksave()
  %soff = getelementptr inbounds [5 x i8*], [5 x i8*]* %ctx, i64 0, i64 2
  store i8* %stack, i8** %soff, align 16

  ; The rest are architecture specific and stored by setjmp().
  %buff = bitcast [5 x i8*]* %ctx to i8*
  %retv = call i32 @llvm.eh.sjlj.setjmp(i8* %buff) returns_twice
  ret i32 %retv
}

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
@jump_swap([5 x i8*]* nonnull %from, [5 x i8*]* nonnull %into)
nounwind
{
  ; setjmp(%from)
  %retv = call i32 @jump_save([5 x i8*]* %from) returns_twice
  %zero = icmp eq i32 %retv, 0
  br i1 %zero, label %jump, label %done

jump:                                        ; setjmp(%from) returned 0
  %ibuf = bitcast [5 x i8*]* %into to i8*
  call void @llvm.eh.sjlj.longjmp(i8* %ibuf) ; longjmp(%into)
  unreachable

done:                                        ; setjmp(%from) returned !0
  ret void
}

; This function sets up the coroutine. It does the following steps:
;   1. Call setjmp().
;   2. Set the stack pointer to %addr.
;   3. Call %func(%c, %f).
define dso_local void
@jump_init(i8* %addr, i8* %c, i8* %f, void (i8**, i8*, i8*)* %func)
nounwind
{

  %buff = alloca [5 x i8*], align 16          ; Allocate setjmp() buffer
  %casti = bitcast [5 x i8*]* %buff to i8*
  %Size = getelementptr inbounds [5 x i8*], [5 x i8*]* null, i32 1
  %SizeI = ptrtoint [5 x i8*]* %Size to i64
  call void @llvm.lifetime.start(i64 %SizeI, i8* nonnull %casti)

  ; Call setjmp(%buff)
  %retv = tail call i32 @jump_save([5 x i8*]* %buff) returns_twice
  %zero = icmp eq i32 %retv, 0
  br i1 %zero, label %next, label %done

next:                                         ; setjmp(%buff) returned 0
  call void @llvm.stackrestore(i8* %addr)     ; Move onto new stack %addr
  %cast = bitcast [5 x i8*]* %buff to i8**
  call void %func(i8** %cast, i8* %c, i8* %f) ; Call %func(%buff, %c, %f)
  unreachable

done:                                         ; setjmp(%buff) returned !0
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