using System.Windows;
using System.Windows.Controls;

namespace Eiviz.Host;

internal static class OnOffLook
{
    public static void Apply(Control control, bool on)
    {
        control.SetResourceReference(Control.BackgroundProperty, on ? "ToggleOnBackground" : "ToggleOffBackground");
        control.SetResourceReference(Control.BorderBrushProperty, on ? "ToggleOnBorder" : "ToggleOffBorder");
        control.SetResourceReference(Control.ForegroundProperty, on ? "ToggleOnForeground" : "ToggleOffForeground");
        control.FontWeight = on ? FontWeights.Bold : FontWeights.Normal;
    }
}
