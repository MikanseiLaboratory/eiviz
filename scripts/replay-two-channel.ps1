# Drives the running host through the two-channel replay workflow the spec asks for:
# add Replay A and B, configure a drive and an angle, record, create an event on each
# channel, and check that each clip lands on its own channel's playlist.
#
# Everything goes through the real UI (UI Automation) or the real API.

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
using System.Collections.Generic;
public class DeckWin {
    public delegate bool Cb(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(Cb cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int c);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    public static IntPtr Find(uint pid, string title) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h)) {
                var sb = new StringBuilder(300); GetWindowTextW(h, sb, 300);
                if (sb.ToString() == title) { found = h; return false; }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@ -ErrorAction SilentlyContinue

$pidHost = [uint32]((Get-Process Eiviz.Host | Select-Object -First 1).Id)

function Element([IntPtr]$w, [string]$id) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($w)
    $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
        (New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::AutomationIdProperty, $id)))
}

function Click([IntPtr]$w, [string]$id) { (Element $w $id).GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() }
function TextOf([IntPtr]$w, [string]$id) { (Element $w $id).Current.Name }

function WaitWindow([string]$title, [int]$seconds = 15) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        $h = [DeckWin]::Find($pidHost, $title)
        if ($h -ne [IntPtr]::Zero) { return $h }
        Start-Sleep -Milliseconds 300
    }
    throw "window not found: $title"
}

function AddReplayInput([IntPtr]$main) {
    Click $main "AddInputButton"
    $picker = WaitWindow "Input Select"
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($picker)
    $buttons = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        (New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::Button)))
    for ($i = 0; $i -lt $buttons.Count; $i++) {
        if ($buttons.Item($i).Current.Name -eq "Replay") {
            $buttons.Item($i).GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
            break
        }
    }
    Start-Sleep -Milliseconds 700
    Click $picker "OkButton"
    Start-Sleep -Seconds 4
}

function Rows([IntPtr]$w, [string]$id) {
    (Element $w $id).FindAll([System.Windows.Automation.TreeScope]::Descendants,
        (New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::DataItem))).Count
}

$main = WaitWindow "eiviz Professional"
"main: $main"
AddReplayInput $main
"input A added"
AddReplayInput $main
"input B added"

# Replay Setup: one drive, one angle.
Click $main "ReplayButton"
$deck = WaitWindow "インスタントリプレイ"
Click $deck "OpenSetupButton"
$setup = WaitWindow "リプレイ設定"
$root = [System.Windows.Automation.AutomationElement]::FromHandle($setup)
$buttons = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button)))
for ($i = 0; $i -lt $buttons.Count; $i++) {
    if ($buttons.Item($i).Current.Name -like "*保存先を追加*") {
        $buttons.Item($i).GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
        break
    }
}
$browser = WaitWindow "録画先のドライブを選択"
[DeckWin]::SetForegroundWindow($browser) | Out-Null
Start-Sleep -Milliseconds 900
[System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
Start-Sleep -Seconds 3
"drive: $(TextOf $setup 'StatusText')"

Click $setup "AddAngleButton"
Start-Sleep -Seconds 2
"angle: $(TextOf $setup 'StatusText')"

# Auto-add on, so creating an event also files it on the channel's own playlist.
$auto = Element $setup "AutoAddBox"
$toggle = $auto.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
if ($toggle.Current.ToggleState -ne "On") { $toggle.Toggle(); Start-Sleep -Milliseconds 400 }
Click $setup "ApplyButton"
Start-Sleep -Seconds 3
"applied: $(TextOf $setup 'StatusText')"

# Record.
$api = "http://127.0.0.1:8088/API/"
(Invoke-WebRequest -Uri "$api`?Function=ReplayStartRecording" -TimeoutSec 20 -UseBasicParsing -SkipHttpErrorCheck) | Out-Null
Start-Sleep -Seconds 10
"state: $(TextOf $deck 'StateText')"

# Channel A: an event, which must land on A's playlist.
Click $deck "MarkInButton"; Start-Sleep -Seconds 3
Click $deck "MarkOutButton"; Start-Sleep -Seconds 2
"A clips: $(Rows $deck 'PlaylistList')  playlist: $((Element $deck 'PlaylistBox').GetCurrentPattern([System.Windows.Automation.SelectionPattern]::Pattern).Current.GetSelection()[0].Current.Name)"

# Channel B: the same, which must land on B's own playlist.
$channelBox = Element $deck "ChannelBox"
$channelBox.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
Start-Sleep -Milliseconds 700
$options = $channelBox.FindAll([System.Windows.Automation.TreeScope]::Descendants,
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::ListItem)))
$options.Item(1).GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()
Start-Sleep -Seconds 2
"on B: $((Element $deck 'OutputLabel').Current.Name)"
Click $deck "MarkInButton"; Start-Sleep -Seconds 3
Click $deck "MarkOutButton"; Start-Sleep -Seconds 2
"B clips: $(Rows $deck 'PlaylistList')  playlist: $((Element $deck 'PlaylistBox').GetCurrentPattern([System.Windows.Automation.SelectionPattern]::Pattern).Current.GetSelection()[0].Current.Name)"

# Play B: only B should move.
Click $deck "PlaylistPlayButton"
Start-Sleep -Seconds 1
$a = TextOf $deck "PlayheadTcText"
Start-Sleep -Seconds 2
"B playhead: $a -> $(TextOf $deck 'PlayheadTcText')  output: $(TextOf $deck 'OutputLabel')"

"deck: $deck"
