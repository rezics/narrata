using Narrata.Protocol.V1;

namespace Narrata.Unity;

public interface INarrataPresenter
{
    void Present(Result result);
}

public interface INarrataSceneReconciler
{
    // Reconcile the host scene to the complete target returned by the protocol.
    void Reconcile(CommittedRunResult committed);
}

public interface INarrataHostSaveCoordinator
{
    // Persist the host snapshot before publishing the matching Narrata compound-save reference.
    ValueTask SaveTogetherAsync(byte[] checkpointBundle, CancellationToken cancellationToken);
}
