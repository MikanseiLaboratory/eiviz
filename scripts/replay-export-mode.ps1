# Exports one file through the running host's export window, using the real dialog.
# Confirms the recording warning by its access key, not by guessing which window is in
# front, so a stale modal can never swallow the keystroke and start a second export.

param(
    [Parameter(Mandatory = $true)][int]$ModeIndex,
    [Parameter(Mandatory = $true)][string]$Path,
    [int]$TimeoutSeconds = 60
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
using System.Collections.Generic;
public class ExWin {
    public delegate bool Cb(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(Cb cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int c);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    public static List<string> Titles(uint pid) {
        var r = new List<string>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h)) {
                var sb = new StringBuilder(300); GetWindowTextW(h, sb, 300);
                if (sb.Length > 0) r.Add(sb.ToString());
            }
            return true;
        }, IntPtr.Zero);
        return r;
    }
    public static List<IntPtr> All(uint pid, string title) {
        var r = new List<IntPtr>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h)) {
                var sb = new StringBuilder(300); GetWindowTextW(h, sb, 300);
                if (sb.ToString() == title) r.Add(h);
            }
            return true;
        }, IntPtr.Zero);
        return r;
    }
}
'@ -ErrorAction SilentlyContinue

$pidHost = (Get-Process Eiviz.Host | Select-Object -First 1).Id

function Element([IntPtr]$w, [string]$id) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($w)
    $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
        (New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::AutomationIdProperty, $id)))
}

# The export window is the LAST "書き出し" window; any earlier ones are leftover dialogs.
$titles = [ExWin]::Titles([uint32]$pidHost)
$exportWindows = [ExWin]::All([uint32]$pidHost, "書き出し")
if ($exportWindows.Count -eq 0) { throw "no export window is open" }
$ex = $exportWindows[0]
"export window: $ex  (open '書き出し' windows: $($exportWindows.Count))"
if ($exportWindows.Count -gt 1) { throw "stray dialogs are open; dismiss them first" }

$start = Element $ex "ExportStartButton"
if (-not $start.Current.IsEnabled) { throw "export is already running" }

$box = Element $ex "ExportModeBox"
$box.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern).Expand() | Out-Null
Start-Sleep -Milliseconds 700
$items = $box.FindAll([System.Windows.Automation.TreeScope]::Descendants,
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::ListItem)))
if ($ModeIndex -ge $items.Count) { throw "mode $ModeIndex does not exist" }
$chosen = $items.Item($ModeIndex).Current.Name
$items.Item($ModeIndex).GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()
Start-Sleep -Milliseconds 500
$box.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern).Collapse() | Out-Null
"mode: $chosen"

(Element $ex "ExportPathBox").GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($Path)
Start-Sleep -Milliseconds 400

$start.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
Start-Sleep -Seconds 2

# A recording is still running, so the window asks first. Find the dialog that is not the
# export window, focus it by handle, and answer with its OK access key.
foreach ($h in [ExWin]::All([uint32]$pidHost, "書き出し")) {
    if ($h -eq $ex) { continue }
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($h)
    $panes = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition)
    $isWarning = $false
    for ($i = 0; $i -lt $panes.Count; $i++) {
        if ($panes.Item($i).Current.Name -like "*録画中*") { $isWarning = $true; break }
    }
    if (-not $isWarning) { continue }
    [ExWin]::SetForegroundWindow($h) | Out-Null
    Start-Sleep -Milliseconds 700
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.SendKeys]::SendWait("%o")
    Start-Sleep -Milliseconds 300
    [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
    "answered the recording warning"
    Start-Sleep -Seconds 1
    break
}

$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
$last = ""
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 2
    $status = (Element $ex "ExportStatusText").Current.Name
    if ($status -ne $last) { "  status: $status"; $last = $status }
    if ($status.Length -gt 0 -and -not (Element $ex "ExportStartButton").Current.IsEnabled) { continue }
    if ($status -match '完了|error|失敗|中止|キャンセル') { break }
}
"final: $last"
"file: $(Test-Path $Path)"
