#if UNITY_WEBGL && !UNITY_EDITOR
using System.Runtime.InteropServices;

namespace Narrata.Unity;

public static class NarrataWebClient
{
    [DllImport("__Internal")]
    private static extern int NarrataWebBridge_Call(
        byte[] request,
        int requestLength,
        out IntPtr response,
        out int responseLength);

    [DllImport("__Internal")]
    private static extern void NarrataWebBridge_Free(IntPtr response);

    public static byte[] Call(byte[] request)
    {
        var status = NarrataWebBridge_Call(request, request.Length, out var pointer, out var length);
        if (status != 0) throw new InvalidOperationException("Narrata Web could not process the request.");
        try
        {
            var response = new byte[length];
            Marshal.Copy(pointer, response, 0, length);
            return response;
        }
        finally
        {
            NarrataWebBridge_Free(pointer);
        }
    }
}
#endif
