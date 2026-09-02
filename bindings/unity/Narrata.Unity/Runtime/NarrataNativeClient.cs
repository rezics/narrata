#if !UNITY_WEBGL || UNITY_EDITOR
using Narrata.Binding;
using Narrata.Protocol.V1;

namespace Narrata.Unity;

public sealed class NarrataNativeClient : IDisposable
{
    private readonly NarrataEngine _engine = new();

    public Response Call(Request request) => _engine.Call(request);

    public void Dispose() => _engine.Dispose();
}
#endif
