using AiUsageBar.Windows;
using System.IO;

var tests = new (string Name, Action Body)[] {
    ("parses ordered report sections", ParsesOrderedReportSections),
    ("selects configured primary entry", SelectsConfiguredPrimaryEntry),
    ("falls back to first ready entry", FallsBackToFirstReadyEntry),
    ("rejects missing entries", RejectsMissingEntries),
};

var failures = 0;
foreach (var (name, body) in tests)
{
    try
    {
        body();
        Console.WriteLine($"PASS {name}");
    }
    catch (Exception exception)
    {
        failures++;
        Console.Error.WriteLine($"FAIL {name}: {exception.Message}");
    }
}

return failures == 0 ? 0 : 1;

static void ParsesOrderedReportSections()
{
    const string json = """
        {"primary":"zenmux","entries":[{"id":"zenmux","display_name":"ZenMux","short_name":"zmx","plan":"Pro","status":"ready","error":null,"stale":false,"fetched_at":"2026-08-28T12:00:00Z","sections":[{"type":"text","label":"Balance","value":"$42.50"},{"type":"metric","label":"5h quota","percent":84,"value":"840 / 1000","detail":"reset 1h","severity":"high","reset_at":"2026-08-28T13:00:00Z"}]}]}
        """;
    var report = UsageReportParser.Parse(json);
    Assert(report.Entries.Count == 1, "entry count");
    Assert(report.Entries[0].Sections.GetArrayLength() == 2, "ordered sections");
    Assert(report.Entries[0].FetchedAt is not null, "timestamp parsed");
}

static void SelectsConfiguredPrimaryEntry()
{
    var report = UsageReportParser.Parse("""
        {"primary":"second","entries":[{"id":"first","display_name":"First","short_name":"fst","status":"ready","stale":false,"sections":[]},{"id":"second","display_name":"Second","short_name":"snd","status":"error","stale":false,"sections":[]}]}
        """);
    Assert(UsageReportParser.SelectPrimary(report)?.Id == "second", "primary id");
}

static void FallsBackToFirstReadyEntry()
{
    var report = UsageReportParser.Parse("""
        {"primary":"missing","entries":[{"id":"error","display_name":"Error","short_name":"err","status":"error","stale":false,"sections":[]},{"id":"ready","display_name":"Ready","short_name":"rdy","status":"ready","stale":false,"sections":[]}]}
        """);
    Assert(UsageReportParser.SelectPrimary(report)?.Id == "ready", "first ready fallback");
}

static void RejectsMissingEntries()
{
    AssertThrows(() => UsageReportParser.Parse("{}"), "missing entries");
}

static void Assert(bool condition, string message)
{
    if (!condition)
    {
        throw new InvalidOperationException(message);
    }
}

static void AssertThrows(Action action, string message)
{
    try
    {
        action();
    }
    catch (InvalidDataException)
    {
        return;
    }
    throw new InvalidOperationException(message);
}
