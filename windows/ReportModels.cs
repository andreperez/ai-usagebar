using System.IO;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace AiUsageBar.Windows;

public sealed record UsageReport(
    [property: JsonPropertyName("primary")] string? Primary,
    [property: JsonPropertyName("entries")] IReadOnlyList<UsageEntry> Entries);

public sealed record UsageEntry(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("display_name")] string DisplayName,
    [property: JsonPropertyName("short_name")] string ShortName,
    [property: JsonPropertyName("plan")] string? Plan,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("error")] string? Error,
    [property: JsonPropertyName("stale")] bool Stale,
    [property: JsonPropertyName("fetched_at")] DateTimeOffset? FetchedAt,
    [property: JsonPropertyName("sections")] JsonElement Sections)
{
    public string Title
    {
        get
        {
            if (string.IsNullOrWhiteSpace(Plan) || string.Equals(Plan, DisplayName, StringComparison.Ordinal))
            {
                return DisplayName;
            }

            return Plan.StartsWith($"{DisplayName} · ", StringComparison.Ordinal)
                || Plan.StartsWith($"{DisplayName} — ", StringComparison.Ordinal)
                ? Plan
                : $"{DisplayName} - {Plan}";
        }
    }
}

public static class UsageReportParser
{
    private static readonly JsonSerializerOptions Options = new() { PropertyNameCaseInsensitive = true };

    public static UsageReport Parse(string json)
    {
        var report = JsonSerializer.Deserialize<UsageReport>(json, Options);
        if (report is null || report.Entries is null)
        {
            throw new InvalidDataException("ai-usagebar returned an invalid usage report.");
        }

        return report;
    }

    public static UsageEntry? SelectPrimary(UsageReport report)
    {
        return report.Entries.FirstOrDefault(entry => entry.Id == report.Primary)
            ?? report.Entries.FirstOrDefault(entry => entry.Status == "ready")
            ?? report.Entries.FirstOrDefault();
    }
}
