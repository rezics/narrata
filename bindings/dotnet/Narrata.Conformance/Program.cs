using Google.Protobuf;
using Narrata.Binding;
using Narrata.Protocol.V1;

if (args.Length != 1)
    throw new ArgumentException("Pass the frozen Program Artifact hex fixture path.");

var artifact = Convert.FromHexString(File.ReadAllText(args[0]).Trim());
using var engine = new NarrataEngine();

var loaded = engine.Call(new Request
{
    ProtocolVersion = 1,
    RequestId = 1,
    ProgramLoad = new ProgramLoad { Artifact = ByteString.CopyFrom(artifact) },
});
if (loaded.BodyCase != Response.BodyOneofCase.ProgramLoaded || loaded.ProgramLoaded.ArtifactId.Length != 32)
    throw new InvalidOperationException("ProgramLoad did not return a checked Artifact ID.");

var execution = new byte[16];
execution[^1] = 1;
var created = engine.Call(new Request
{
    ProtocolVersion = 1,
    RequestId = 2,
    SessionCreate = new SessionCreate
    {
        ArtifactId = loaded.ProgramLoaded.ArtifactId,
        ExecutionId = ByteString.CopyFrom(execution),
    },
});
if (created.BodyCase != Response.BodyOneofCase.SessionCreated)
    throw new InvalidOperationException("SessionCreate did not return a session.");

var input = new byte[16];
input[^1] = 1;
var committed = engine.Call(new Request
{
    ProtocolVersion = 1,
    RequestId = 3,
    Dispatch = new Dispatch
    {
        Session = created.SessionCreated.Session,
        SliceWork = 100_000,
        Input = new RuntimeInput
        {
            RequestId = ByteString.CopyFrom(input),
            Start = new Start(),
        },
    },
});
if (committed.BodyCase != Response.BodyOneofCase.Committed
    || committed.Committed.CommitId.Length != 32
    || committed.Committed.ReceiptId.Length != 32
    || committed.Committed.StateDigest.Length != 32)
    throw new InvalidOperationException("Dispatch did not commit the shared conformance trace.");

Console.WriteLine(Convert.ToHexString(committed.Committed.StateDigest.Span).ToLowerInvariant());
