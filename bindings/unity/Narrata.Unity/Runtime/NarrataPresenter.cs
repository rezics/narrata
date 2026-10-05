using Narrata.Protocol.V2;

namespace Narrata.Unity;

public interface INarrataPresenter
{
    // The result names content references, never text (ADR 0018); resolve them with an
    // INarrataContentResolver before showing the speaker, body, prompt and choice labels.
    void Present(Result result);
}

public interface INarrataContentResolver
{
    // Resolves one screen's references in a single batch with one language context. A
    // ContentRef is passed as a Segment whose Unit is the reference. A null entry means the
    // content is unavailable or incompatible; the host decides what to show instead.
    ValueTask<IReadOnlyList<string?>> ResolveAsync(
        IReadOnlyList<Segment> contents,
        CancellationToken cancellationToken);
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
