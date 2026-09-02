using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace Narrata.Binding;

internal enum NarStatus
{
    Ok = 0,
    InvalidArgument = 1,
    InvalidHandle = 2,
    ProtocolError = 3,
    Panic = 4,
    Internal = 5,
}

[StructLayout(LayoutKind.Sequential)]
internal readonly struct NativeBuffer
{
    internal readonly nint Data;
    internal readonly nuint Length;
    internal readonly ulong Token;
}

internal sealed class NarrataEngineHandle : SafeHandleZeroOrMinusOneIsInvalid
{
    private NarrataEngineHandle() : base(true) { }

    internal NarrataEngineHandle(ulong value) : base(true)
    {
        SetHandle(checked((nint)value));
    }

    protected override bool ReleaseHandle() =>
        NativeMethods.EngineFree(checked((ulong)handle)) == NarStatus.Ok;
}

internal static partial class NativeMethods
{
    private const string Library = "narrata_ffi";

    [LibraryImport(Library, EntryPoint = "nar_abi_version")]
    internal static partial uint AbiVersion();

    [LibraryImport(Library, EntryPoint = "nar_engine_create")]
    internal static partial NarStatus EngineCreate(out ulong handle);

    [LibraryImport(Library, EntryPoint = "nar_engine_free")]
    internal static partial NarStatus EngineFree(ulong handle);

    [LibraryImport(Library, EntryPoint = "nar_engine_call")]
    internal static unsafe partial NarStatus EngineCall(
        ulong handle,
        byte* input,
        nuint inputLength,
        out NativeBuffer output);

    [LibraryImport(Library, EntryPoint = "nar_buffer_free")]
    internal static partial NarStatus BufferFree(ulong token);
}
