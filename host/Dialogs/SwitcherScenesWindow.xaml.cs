using System.Windows;
using System.Windows.Controls;
using Eiviz.Host.I18n;

namespace Eiviz.Host.Dialogs;

public partial class SwitcherScenesWindow : Window
{
    private readonly MixingUnitEntry _unit;
    private readonly Session _session;
    private bool _suppress;

    public SwitcherScenesWindow(MixingUnitEntry unit, Session session)
    {
        InitializeComponent();
        _unit = unit;
        _session = session;
        Title = Loc.T("switcher.manageScenes");
        AllBox.Content = Loc.T("switcher.allScenes");
        IncludeBox.Content = Loc.T("switcher.onlyThese");
        ExcludeBox.Content = Loc.T("switcher.hideThese");
        _suppress = true;
        AllBox.IsChecked = unit.SwitcherSceneFilter == SwitcherSceneFilter.All;
        IncludeBox.IsChecked = unit.SwitcherSceneFilter == SwitcherSceneFilter.Include;
        ExcludeBox.IsChecked = unit.SwitcherSceneFilter == SwitcherSceneFilter.Exclude;
        _suppress = false;
        FillList();
    }

    private SwitcherSceneFilter SelectedFilter =>
        IncludeBox.IsChecked == true
            ? SwitcherSceneFilter.Include
            : ExcludeBox.IsChecked == true
                ? SwitcherSceneFilter.Exclude
                : SwitcherSceneFilter.All;

    private void Mode_Changed(object sender, RoutedEventArgs e)
    {
        if (_suppress)
            return;
        FillList();
    }

    private void FillList()
    {
        SceneList.Items.Clear();
        var filter = SelectedFilter;
        SceneList.IsHitTestVisible = filter != SwitcherSceneFilter.All;
        SceneList.Opacity = filter == SwitcherSceneFilter.All ? 0.65 : 1;
        foreach (var scene in _session.Scenes)
        {
            var selected = filter == SwitcherSceneFilter.All
                || _unit.SwitcherSceneIds.Contains(scene.Id);
            SceneList.Items.Add(new CheckBox
            {
                Content = scene.Name,
                Tag = scene.Id,
                IsChecked = selected,
                Foreground = System.Windows.Media.Brushes.WhiteSmoke,
                Margin = new Thickness(0, 2, 0, 2)
            });
        }
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        _unit.SwitcherSceneFilter = SelectedFilter;
        _unit.SwitcherSceneIds.Clear();
        if (_unit.SwitcherSceneFilter != SwitcherSceneFilter.All)
        {
            foreach (CheckBox box in SceneList.Items)
            {
                if (box.IsChecked == true && box.Tag is ulong id)
                    _unit.SwitcherSceneIds.Add(id);
            }
        }
        DialogResult = true;
    }
}
