using System.Windows;

namespace AiUsageBar.Windows;

public partial class App : System.Windows.Application
{
    protected override void OnStartup(StartupEventArgs eventArgs)
    {
        base.OnStartup(eventArgs);
        var window = new MainWindow(eventArgs.Args.FirstOrDefault());
        MainWindow = window;
        window.Show();
    }
}
