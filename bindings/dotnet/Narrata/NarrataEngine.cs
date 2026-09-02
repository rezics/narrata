using Google.Protobuf;
using Narrata.Protocol.V1;

namespace Narrata.Binding;

public sealed class NarrataException : Exception
{
    public string Code { get; }

    internal NarrataException(string code, string message) : base(message) => Code = code;
}

/// <summary>
/// Owns one synchronized native engine. Requests are copied into the native call and responses
/// are copied out before the native buffer is released, so callers never manage ownership tokens.
/// </summary>
public sealed class NarrataEngine : IDisposable
{
    public const uint RequiredAbiVersion = 1;
    private readonly NarrataEngineHandle _handle;

    public NarrataEngine()
    {
        var actual = NativeMethods.AbiVersion();
        if (actual != RequiredAbiVersion)
            throw new NarrataException("NAR-CS0001", $"Narrata ABI {actual} is incompatible with required ABI {RequiredAbiVersion}.");
        ThrowStatus(NativeMethods.EngineCreate(out var handle));
        _handle = new NarrataEngineHandle(handle);
    }

    public unsafe Response Call(Request request)
    {
        ObjectDisposedException.ThrowIf(_handle.IsClosed, this);
        var input = request.ToByteArray();
        NativeBuffer native;
        NarStatus status;
        fixed (byte* pointer = input)
        {
            status = NativeMethods.EngineCall(
                checked((ulong)_handle.DangerousGetHandle()),
                pointer,
                checked((nuint)input.Length),
                out native);
        }

        try
        {
            if (native.Token != 0)
            {
                var length = checked((int)native.Length);
                var output = new byte[length];
                if (length != 0)
                    System.Runtime.InteropServices.Marshal.Copy(native.Data, output, 0, length);
                var response = Response.Parser.ParseFrom(output);
                if (response.BodyCase == Response.BodyOneofCase.Diagnostic)
                    throw new NarrataException(response.Diagnostic.Code, response.Diagnostic.Message);
                ThrowStatus(status);
                return response;
            }

            ThrowStatus(status);
            throw new NarrataException("NAR-CS0002", "Narrata returned no response.");
        }
        finally
        {
            if (native.Token != 0)
                ThrowStatus(NativeMethods.BufferFree(native.Token));
        }
    }

    /// <summary>
    /// Schedules a pull-protocol call without retaining a native continuation in managed code.
    /// </summary>
    public ValueTask<Response> CallAsync(Request request, CancellationToken cancellationToken = default)
    {
        cancellationToken.ThrowIfCancellationRequested();
        return ValueTask.FromResult(Call(request));
    }

    public void Dispose() => _handle.Dispose();

    private static void ThrowStatus(NarStatus status)
    {
        if (status == NarStatus.Ok) return;
        throw new NarrataException(
            $"NAR-CS{1000 + (int)status}",
            status switch
            {
                NarStatus.InvalidArgument => "The native call received an invalid argument.",
                NarStatus.InvalidHandle => "The Narrata engine handle is no longer valid.",
                NarStatus.ProtocolError => "The request is not a valid Narrata protocol message.",
                NarStatus.Panic => "Narrata contained an internal failure at the native boundary.",
                _ => "Narrata could not complete the native call.",
            });
    }
}
