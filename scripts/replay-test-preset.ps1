# Brings the running eiviz up to the state every replay test starts from, from whatever
# session happens to be open.
#
# The session file is a binary container, so it cannot be edited as text and a save dialog
# is hard to drive reliably. What *can* be reproduced exactly is the sequence of operator
# actions that produces the state, so that is what this does — and it is the same sequence
# a person would follow, which makes it a better test of the product than a hand-edited
# document would be.
#
# What it leaves in place:
#   Inputs     1. Color Red  2. SMPTE HD Bars  3. Black  4. Blue
#              5. OBS PGM (OMT, from OBS's own output)
#              6. Replay A
#   Scene 1    SMPTE bars on program, so the replay records a picture with a known look
#   Replay     drive D:\replay\test1, one angle on Scene 1 recording its own audio,
#              auto-add on, clip order append
#
# Run it with eiviz already open. It is idempotent: a session that is already set up is
# left alone.

param(
    [string]$Api = "http://127.0.0.1:8088/API/",
    [string]$RecordingPath = "D:\replay\test1",
    [string]$OmtName = "OBS Output",
    [switch]$KeepRecording
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes

Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
using System.Collections.Generic;
public class PresetWin {
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

function Click([IntPtr]$w, [string]$id) {
    $e = Element $w $id
    if (-not $e) { throw "no control $id" }
    $e.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
}

function WaitWindow([string]$title, [int]$seconds = 15) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        $h = [PresetWin]::Find($pidHost, $title)
        if ($h -ne [IntPtr]::Zero) { return $h }
        Start-Sleep -Milliseconds 300
    }
    throw "window not found: $title"
}

function Api([string]$function) {
    (Invoke-WebRequest -Uri "$Api`?Function=$function" -TimeoutSec 20 -UseBasicParsing -SkipHttpErrorCheck).Content
}

function Inputs { [regex]::Matches((Invoke-WebRequest -Uri $Api -TimeoutSec 10 -UseBasicParsing).Content, 'type="(\w+)" title="([^"]*)"') }

$main = WaitWindow "eiviz Professional"
"main: $main"
$existing = Inputs
"inputs now: $(($existing | ForEach-Object { $_.Groups[2].Value }) -join ', ')"

# 1. The OMT input from OBS, if it is not already there and OBS is advertising one.
$hasOmt = $existing | Where-Object { $_.Groups[1].Value -eq "OMT" }
if (-not $hasOmt) {
    Click $main "AddInputButton"
    $picker = WaitWindow "Input Select"
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($picker)
    $kinds = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        (New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::Button)))
    for ($i = 0; $i -lt $kinds.Count; $i++) {
        if ($kinds.Item($i).Current.Name -eq "OMT") {
            $kinds.Item($i).GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
            break
        }
    }
    Start-Sleep -Seconds 2
    $list = Element $picker "OmtList"
    if (-not $list) { throw "no OMT discovery list" }
    $items = $list.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        (New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::ListItem)))
    $picked = $null
    for ($i = 0; $i -lt $items.Count; $i++) {
        if ($items.Item($i).Current.Name -match [regex]::Escape($OmtName)) {
            $picked = $items.Item($i)
            break
        }
    }
    if (-not $picked) { throw "OBS is not advertising an OMT output called '$OmtName'" }
    $picked.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Seconds 2
    (Element $picker "NameBox").GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue("OBS PGM")
    Click $picker "OkButton"
    Start-Sleep -Seconds 4
    "added the OMT input"
} else {
    "OMT input already present"
}

# 2. Replay setup: a drive, one angle on Scene 1 recording its own audio, auto-add on.
Click $main "ReplayButton"
$deck = WaitWindow "インスタントリプレイ"
Click $deck "OpenSetupButton"
$setup = WaitWindow "リプレイ設定"

$status = (Element $setup "StatusText").Current.Name
$storage = Element $setup "StorageList"
$storageCount = $storage.FindAll([System.Windows.Automation.TreeScope]::Descendants,
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::DataItem))).Count
if ($storageCount -eq 0) {
    $buttons = [System.Windows.Automation.AutomationElement]::FromHandle($setup).FindAll(
        [System.Windows.Automation.TreeScope]::Descendants,
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
    [PresetWin]::SetForegroundWindow($browser) | Out-Null
    Start-Sleep -Milliseconds 900
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
    Start-Sleep -Seconds 3
    "drive: $((Element $setup 'StatusText').Current.Name)"
} else {
    "drive already configured"
}

$angleList = Element $setup "AngleList"
$angleCount = $angleList.FindAll([System.Windows.Automation.TreeScope]::Descendants,
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::DataItem))).Count
if ($angleCount -eq 0) {
    Click $setup "AddAngleButton"
    Start-Sleep -Seconds 2
    "angle: $((Element $setup 'StatusText').Current.Name)"
} else {
    "angle already configured"
}

$auto = Element $setup "AutoAddBox"
$toggle = $auto.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
if ($toggle.Current.ToggleState -ne "On") {
    $toggle.Toggle()
    Start-Sleep -Milliseconds 400
}
"autoAdd: $($toggle.Current.ToggleState)"

Click $setup "ApplyButton"
Start-Sleep -Seconds 3
"applied: $((Element $setup 'StatusText').Current.Name)"

# 3. Bars on program, so the recording has a known picture.
Api "CutDirect&Input=Scene 1" | Out-Null
Start-Sleep -Seconds 1

# 4. The playlist files new clips at the end, which is how the tests expect them.
$order = Element $deck "ClipOrderBox"
if ($order) {
    $options = $order.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        (New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::ListItem)))
    for ($i = 0; $i -lt $options.Count; $i++) {
        if ($options.Item($i).Current.Name -match "末尾|Append") {
            $options.Item($i).GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()
            break
        }
    }
}

if (-not $KeepRecording) {
    Api "ReplayStartRecording" | Out-Null
    Start-Sleep -Seconds 4
}
"state: $((Element $deck 'StateText').Current.Name)"
"preset ready; deck handle $deck"
