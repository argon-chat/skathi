using System.Globalization;
using System.Text.Json;
using System.Text.RegularExpressions;

const string Usage = "Usage: dotnet script medians.csx -- <file.jsonl> --runs <count>";
if (Args.Contains("--help") || Args.Contains("-h"))
{
    Console.WriteLine(Usage);
    return 0;
}

string App(JsonElement row) => row.TryGetProperty("tier", out _)
    ? $"{row.GetProperty("scheme").GetString()}/{row.GetProperty("pressure").GetString()}"
    : row.GetProperty("app").GetString();

string[] Suite(JsonElement first)
{
    if (first.TryGetProperty("tier", out _)) return new[] { "balanced/nominal", "saving/nominal", "saving/critical" };
    if (first.TryGetProperty("result", out _)) return new[] { "skathi-vec", "skathi-slice", "electron", "tauri" };
    if (App(first) == "skathi-full") return new[] { "skathi-full" };
    if (App(first) is "standalone-default" or "standalone-low") return new[] { "standalone-default", "standalone-low" };
    if (first.TryGetProperty("after_scroll", out _)) return new[] { "skathi", "electron", "tauri" };
    return new[] { "jit-default", "jit-low", "jit-unpaced", "aot-default", "aot-compact",
                   "aot-compact-low", "trimmed-compact-low", "full" };
}

double? Reading(JsonElement value) => value.ValueKind == JsonValueKind.Null ? null : value.GetDouble();

Dictionary<string, double?> Metrics(JsonElement row)
{
    var values = new Dictionary<string, double?>();
    if (row.TryGetProperty("result", out var result))
    {
        foreach (Match match in Regex.Matches(result.GetString(), @"(\w+_(?:us|ms))=([\d.]+)"))
            values.Add(match.Groups[1].Value, double.Parse(match.Groups[2].Value, CultureInfo.InvariantCulture));
        return values;
    }
    if (row.TryGetProperty("tier", out _))
    {
        foreach (var key in new[] { "fps", "cpu_pct", "system_cpu_pct" })
            values.Add(key, Reading(row.GetProperty(key)));
        return values;
    }

    values.Add("ready_ms", Reading(row.GetProperty("ready_ms")));
    values.Add("idle_cpu_pct", Reading(row.GetProperty("idle_cpu_pct")));
    foreach (var phase in new[] { "idle", "after_scroll" })
    {
        if (!row.TryGetProperty(phase, out var sample)) continue;
        foreach (var field in sample.EnumerateObject())
            if (field.Name != "names")
                values.Add($"{phase}.{field.Name}", Reading(field.Value));
    }
    if (row.TryGetProperty("after_scroll_cpu_pct", out var cpu))
        values.Add("after_scroll_cpu_pct", Reading(cpu));
    return values;
}

string Median(double?[] series)
{
    if (series.Contains(null)) return "unavailable";
    Array.Sort(series);
    int middle = series.Length / 2;
    double value = series.Length % 2 == 1 ? series[middle].Value : (series[middle - 1].Value + series[middle].Value) / 2;
    return value.ToString(CultureInfo.InvariantCulture);
}

try
{
    string path = null;
    int runs = 0;
    for (int i = 0; i < Args.Count; i++)
    {
        string arg = Args[i];
        if (arg == "--runs" || arg.StartsWith("--runs=", StringComparison.Ordinal))
        {
            string count = arg == "--runs" ? Args.ElementAtOrDefault(++i) : arg.Substring(7);
            if (!int.TryParse(count, out runs) || runs < 1) throw new ArgumentException(Usage);
        }
        else if (path == null && !arg.StartsWith("-")) path = arg;
        else throw new ArgumentException(Usage);
    }
    if (path == null || runs == 0) throw new ArgumentException(Usage);

    var rows = File.ReadLines(path).Where(line => !string.IsNullOrWhiteSpace(line))
        .Select(line => JsonSerializer.Deserialize<JsonElement>(line)).ToArray();
    if (rows.Length == 0) throw new InvalidDataException("Empty measurement file.");
    var apps = Suite(rows[0]);
    var groups = rows.ToLookup(App);
    if (!apps.ToHashSet().SetEquals(groups.Select(group => group.Key)))
        throw new InvalidDataException("Incomplete or mixed suites.");

    foreach (var app in apps)
    {
        var rounds = groups[app].Select(row => row.GetProperty("run").GetInt32()).OrderBy(run => run);
        if (!rounds.SequenceEqual(Enumerable.Range(1, runs)))
            throw new InvalidDataException($"Missing or duplicate rounds for {app}.");
    }
    foreach (var app in apps)
    {
        var samples = groups[app].Select(Metrics).ToArray();
        Console.WriteLine($"{app} (n={samples.Length})");
        foreach (var key in samples[0].Keys)
            Console.WriteLine($"  {key,-32} {Median(samples.Select(sample => sample[key]).ToArray())}");
    }
    return 0;
}
catch (Exception error)
{
    Console.Error.WriteLine(error.Message);
    return 1;
}
