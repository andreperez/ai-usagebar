using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using Forms = System.Windows.Forms;

namespace AiUsageBar.Windows;

public partial class MainWindow : Window
{
    private readonly ReportClient _client;
    private readonly Forms.NotifyIcon _tray;
    private readonly DispatcherTimer _refreshTimer;
    private CancellationTokenSource? _refreshCancellation;
    private UsageReport? _report;
    private bool _shuttingDown;
    private bool _refreshingList;

    public MainWindow(string? binaryPath)
    {
        InitializeComponent();
        _client = new ReportClient(binaryPath);
        _tray = CreateTrayIcon();
        _refreshTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(60) };
        _refreshTimer.Tick += async (_, _) => await RefreshAsync();
        _refreshTimer.Start();
        Loaded += async (_, _) => await RefreshAsync();
        Closed += (_, _) => Shutdown();
    }

    private Forms.NotifyIcon CreateTrayIcon()
    {
        var menu = new Forms.ContextMenuStrip();
        menu.Items.Add("Open dashboard", null, (_, _) => ShowDashboard());
        menu.Items.Add("Refresh", null, async (_, _) => await RefreshAsync());
        menu.Items.Add("Open terminal UI", null, (_, _) => StartTerminalUi());
        menu.Items.Add(new Forms.ToolStripSeparator());
        menu.Items.Add("Exit", null, (_, _) => Shutdown());
        var tray = new Forms.NotifyIcon {
            Icon = SystemIcons.Application,
            Text = "AI Usage Bar",
            ContextMenuStrip = menu,
            Visible = true,
        };
        tray.DoubleClick += (_, _) => ShowDashboard();
        return tray;
    }

    private async void Refresh_Click(object sender, RoutedEventArgs eventArgs)
    {
        await RefreshAsync();
    }

    private void Providers_SelectionChanged(object sender, SelectionChangedEventArgs eventArgs)
    {
        if (_refreshingList)
        {
            return;
        }
        RenderSelection(Providers.SelectedItem as UsageEntry);
    }

    private async Task RefreshAsync()
    {
        if (_refreshCancellation is not null)
        {
            return;
        }

        _refreshCancellation = new CancellationTokenSource();
        RefreshButton.IsEnabled = false;
        Status.Text = "Refreshing...";
        try
        {
            _report = await _client.FetchAsync(_refreshCancellation.Token);
            var primary = UsageReportParser.SelectPrimary(_report);
            _refreshingList = true;
            Providers.ItemsSource = _report.Entries;
            Providers.SelectedItem = primary;
            _refreshingList = false;
            RenderSelection(primary);
            Status.Text = $"Updated {DateTimeOffset.Now:t}";
            _tray.Text = Tooltip(primary);
        }
        catch (OperationCanceledException) when (_shuttingDown)
        {
        }
        catch (Exception exception) when (exception is not OperationCanceledException)
        {
            Status.Text = exception.Message;
            _tray.Text = "AI Usage Bar: refresh failed";
        }
        finally
        {
            _refreshCancellation.Dispose();
            _refreshCancellation = null;
            RefreshButton.IsEnabled = true;
        }
    }

    private void RenderSelection(UsageEntry? entry)
    {
        Details.Children.Clear();
        if (entry is null)
        {
            Details.Children.Add(new TextBlock { Text = "No configured providers.", FontSize = 16 });
            return;
        }

        Details.Children.Add(new TextBlock {
            Text = entry.Title,
            FontSize = 20,
            FontWeight = FontWeights.SemiBold,
            Margin = new Thickness(0, 0, 0, 12),
        });
        if (entry.Error is { Length: > 0 })
        {
            Details.Children.Add(new TextBlock { Text = entry.Error, Foreground = System.Windows.Media.Brushes.OrangeRed, TextWrapping = TextWrapping.Wrap });
            return;
        }
        if (entry.Stale)
        {
            Details.Children.Add(new TextBlock { Text = "Showing cached data.", Foreground = System.Windows.Media.Brushes.Goldenrod, Margin = new Thickness(0, 0, 0, 8) });
        }
        if (entry.Sections.ValueKind != JsonValueKind.Array)
        {
            Details.Children.Add(new TextBlock { Text = "No detail sections were returned." });
            return;
        }

        foreach (var section in entry.Sections.EnumerateArray())
        {
            RenderSection(section);
        }
    }

    private void RenderSection(JsonElement section)
    {
        if (!section.TryGetProperty("type", out var type))
        {
            return;
        }
        if (type.GetString() == "spacer")
        {
            Details.Children.Add(new Border { Height = 8 });
            return;
        }
        var label = section.TryGetProperty("label", out var labelElement) ? labelElement.GetString() : null;
        var value = section.TryGetProperty("value", out var valueElement) ? valueElement.GetString() : null;
        if (type.GetString() == "metric")
        {
            var percent = section.TryGetProperty("percent", out var percentElement) ? percentElement.GetInt32() : 0;
            var detail = section.TryGetProperty("detail", out var detailElement) ? detailElement.GetString() : null;
            value = string.IsNullOrWhiteSpace(detail) ? $"{value} ({percent}%)" : $"{value} ({percent}%)\n{detail}";
        }
        else if (type.GetString() == "block" && section.TryGetProperty("body", out var body) && body.ValueKind == JsonValueKind.Array)
        {
            value = string.Join(Environment.NewLine, body.EnumerateArray().Select(item => item.GetString()));
        }
        Details.Children.Add(new TextBlock {
            Text = string.IsNullOrWhiteSpace(label) ? value : $"{label}: {value}",
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 3, 0, 3),
        });
    }

    private static string Tooltip(UsageEntry? entry)
    {
        if (entry is null)
        {
            return "AI Usage Bar: no configured providers";
        }
        var status = entry.Error is { Length: > 0 } ? "error" : entry.Stale ? "cached" : "ready";
        var text = $"AI Usage Bar: {entry.ShortName} {status}";
        return text.Length <= 127 ? text : text[..127];
    }

    private void ShowDashboard()
    {
        Show();
        WindowState = WindowState.Normal;
        Activate();
    }

    private void StartTerminalUi()
    {
        try
        {
            var binary = ReportClient.ResolveBinary(null);
            var directory = Path.GetDirectoryName(binary);
            if (string.IsNullOrEmpty(directory))
            {
                Status.Text = "Could not resolve the ai-usagebar.exe directory.";
                return;
            }
            var tui = Path.Combine(directory, "ai-usagebar-tui.exe");
            if (!File.Exists(tui))
            {
                Status.Text = "Could not find ai-usagebar-tui.exe beside ai-usagebar.exe.";
                return;
            }
            Process.Start(new ProcessStartInfo { FileName = tui, UseShellExecute = true });
        }
        catch (Exception exception)
        {
            Status.Text = exception.Message;
        }
    }

    private void Shutdown()
    {
        if (_shuttingDown)
        {
            return;
        }
        _shuttingDown = true;
        _refreshTimer.Stop();
        _refreshCancellation?.Cancel();
        _tray.Visible = false;
        _tray.Dispose();
        System.Windows.Application.Current.Shutdown();
    }
}
