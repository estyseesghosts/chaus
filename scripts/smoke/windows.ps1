<#
.SYNOPSIS
    Native Windows smoke test for a packaged chaus executable.

.DESCRIPTION
    Copies the executable and a fixture into a temporary directory and a
    temporary profile, launches it, and drives it with real keyboard/mouse
    input. It proves window creation, focus, typing, clipboard, save, resize,
    Vim toggle, the dirty-close sheet, and (unless -SkipDialogs) the Win32
    open/save dialogs.

    This script controls the real desktop. Run it on a dedicated machine or CI
    runner, never while using the computer.

.PARAMETER Exe
    Path to chaus.exe. It is copied before launch; the checkout is not used as
    the working directory.

.PARAMETER ReportDir
    Directory for smoke-report.json. Defaults to a new directory under TEMP.

.PARAMETER SkipDialogs
    Skip the Open/Save As dialog phase (for runners without a usable desktop
    dialog stack).

.PARAMETER Screenshots
    Save PNG screenshots at the end of each phase. They are evidence only and
    never gate the result.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Exe,

    [string]$ReportDir = "",

    [switch]$SkipDialogs,

    [switch]$Screenshots
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ChausSmokeNative {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, IntPtr e);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
}
'@

$script:ReportDir = if ($ReportDir) {
    [System.IO.Path]::GetFullPath($ReportDir)
} else {
    Join-Path $env:TEMP ("chaus-smoke-" + [Guid]::NewGuid().ToString("N"))
}
$script:Root = Join-Path $env:TEMP ("chaus-smoke-" + [Guid]::NewGuid().ToString("N"))
$script:AppDir = Join-Path $script:Root "app"
$script:WorkDir = Join-Path $script:Root "work"
$script:ProfileDir = Join-Path $script:Root "profile"
$script:Steps = New-Object System.Collections.ArrayList
$script:Processes = New-Object System.Collections.ArrayList
$script:Started = (Get-Date).ToUniversalTime().ToString("o")

function New-SmokeDir([string]$Path) {
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "ASSERT: $Message" }
}

function Add-Step([string]$Name, [string]$Detail) {
    [void]$script:Steps.Add([ordered]@{
        name = $Name
        detail = $Detail
        at = (Get-Date).ToUniversalTime().ToString("o")
    })
    Write-Host "[ok] $Name - $Detail"
}

function Send-Keys([string]$Keys) {
    $shell = New-Object -ComObject WScript.Shell
    $shell.SendKeys($Keys)
    Start-Sleep -Milliseconds 250
}

function Send-Literal([string]$Text) {
    $escaped = [regex]::Replace($Text, '([+^%~()\[\]{}])', '{$1}')
    Send-Keys $escaped
}

function Get-AppRect([System.Diagnostics.Process]$Process) {
    $Process.Refresh()
    Assert-True ($Process.MainWindowHandle -ne [IntPtr]::Zero) "the process has no main window"
    $rect = New-Object ChausSmokeNative+RECT
    Assert-True ([ChausSmokeNative]::GetWindowRect($Process.MainWindowHandle, [ref]$rect)) "GetWindowRect failed"
    return $rect
}

function Move-Window([System.Diagnostics.Process]$Process, [int]$Width, [int]$Height) {
    $rect = Get-AppRect $Process
    $flags = 0x0002 -bor 0x0004 # SWP_NOMOVE | SWP_NOZORDER
    Assert-True ([ChausSmokeNative]::SetWindowPos(
        $Process.MainWindowHandle,
        [IntPtr]::Zero,
        $rect.Left,
        $rect.Top,
        $Width,
        $Height,
        $flags
    )) "SetWindowPos failed"
}

function Focus-App([System.Diagnostics.Process]$Process) {
    $Process.Refresh()
    $handle = $Process.MainWindowHandle
    Assert-True ($handle -ne [IntPtr]::Zero) "cannot focus a process without a window"
    [void][ChausSmokeNative]::ShowWindow($handle, 9)
    [void][ChausSmokeNative]::SetForegroundWindow($handle)
    $shell = New-Object -ComObject WScript.Shell
    [void]$shell.AppActivate($Process.Id)
    Start-Sleep -Milliseconds 400

    $rect = Get-AppRect $Process
    # A click, not just AppActivate, is what reliably gives the editor focus on
    # Windows when the runner has another foreground window.
    [void][ChausSmokeNative]::SetCursorPos(
        [int](($rect.Left + $rect.Right) / 2),
        [int](($rect.Top + $rect.Bottom) / 2)
    )
    [ChausSmokeNative]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    [ChausSmokeNative]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 400

    $foreground = [ChausSmokeNative]::GetForegroundWindow()
    $foregroundPid = [uint32]0
    [void][ChausSmokeNative]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid)
    Assert-True ($foregroundPid -eq $Process.Id) "the app window did not become foreground (got pid $foregroundPid)"
}

function Save-Screenshot([System.Diagnostics.Process]$Process, [string]$Name) {
    if (-not $Screenshots) { return }
    $rect = Get-AppRect $Process
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    New-SmokeDir $script:ReportDir
    $bitmap = New-Object System.Drawing.Bitmap($width, $height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        $bitmap.Save((Join-Path $script:ReportDir $Name), [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

function Wait-AppWindow([System.Diagnostics.Process]$Process, [string]$ExpectedTitle, [int]$Seconds = 20) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 200
        if ($Process.HasExited) {
            throw "the app exited before showing a window (exit code $($Process.ExitCode))"
        }
        $Process.Refresh()
        if ($Process.MainWindowHandle -ne [IntPtr]::Zero -and $Process.MainWindowTitle -eq $ExpectedTitle) {
            return
        }
    }
    $Process.Refresh()
    throw "the window titled '$ExpectedTitle' did not appear (title was '$($Process.MainWindowTitle)')"
}

function Start-SmokeApp([string]$Title, [string]$Argument = "") {
    New-SmokeDir $script:AppDir
    New-SmokeDir $script:WorkDir
    New-SmokeDir $script:ProfileDir
    New-SmokeDir (Join-Path $script:ProfileDir "AppData")
    New-SmokeDir (Join-Path $script:ProfileDir "LocalAppData")

    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = Join-Path $script:AppDir "chaus.exe"
    $info.WorkingDirectory = $script:WorkDir
    $info.UseShellExecute = $false
    if ($Argument) { $info.Arguments = '"' + $Argument + '"' }
    $info.EnvironmentVariables["USERPROFILE"] = $script:ProfileDir
    $info.EnvironmentVariables["HOME"] = $script:ProfileDir
    $info.EnvironmentVariables["APPDATA"] = (Join-Path $script:ProfileDir "AppData")
    $info.EnvironmentVariables["LOCALAPPDATA"] = (Join-Path $script:ProfileDir "LocalAppData")

    $process = [System.Diagnostics.Process]::Start($info)
    [void]$script:Processes.Add($process)
    Wait-AppWindow $process $Title
    Focus-App $process
    return $process
}

function Close-App([System.Diagnostics.Process]$Process, [int]$Seconds = 15) {
    Send-Keys "%{F4}"
    if (-not $Process.WaitForExit($Seconds * 1000)) {
        throw "the app did not exit after Alt+F4"
    }
    Assert-True ($Process.ExitCode -eq 0) "the app exited with code $($Process.ExitCode)"
}

function Read-Fixture([string]$Name) {
    return Get-Content -Raw (Join-Path $script:WorkDir $Name)
}

function Write-Fixture([string]$Name, [string]$Text) {
    $path = Join-Path $script:WorkDir $Name
    [System.IO.File]::WriteAllText($path, $Text)
    return $path
}

function Wait-FileText([string]$Path, [scriptblock]$Predicate, [int]$Seconds = 10) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path $Path) {
            $text = Get-Content -Raw $Path
            if (& $Predicate $text) { return $text }
        }
        Start-Sleep -Milliseconds 150
    }
    throw "the file at $Path did not reach the expected content"
}

function Count-Text([string]$Text, [string]$Needle) {
    return ([regex]::Matches($Text, [regex]::Escape($Needle))).Count
}

function New-Marker() {
    return "SMOKE" + (Get-Date).ToUniversalTime().ToString("HHmmss")
}

function Run-Smoke() {
    New-SmokeDir $script:Root
    New-SmokeDir $script:AppDir
    New-SmokeDir $script:WorkDir
    New-SmokeDir $script:ReportDir
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $script:AppDir "chaus.exe") -Force
    Assert-True (Test-Path (Join-Path $script:AppDir "chaus.exe")) "the executable was not copied"

    $marker = New-Marker
    $path = Write-Fixture "sample.txt" "titlebar smoke"
    $app = Start-SmokeApp "sample.txt" $path
    $rect = Get-AppRect $app
    Assert-True (($rect.Right - $rect.Left) -ge 400) "the window is narrower than its 420 px minimum"
    Add-Step "window" "opened sample.txt at $($rect.Right - $rect.Left)x$($rect.Bottom - $rect.Top)"

    Send-Keys "^{END}"
    Send-Literal $marker
    Send-Keys "^s"
    $text = Wait-FileText $path { param($value) $value.Contains($marker) }
    Add-Step "typing-save" "typed and saved $marker"

    Send-Keys "^a"
    Send-Keys "^c"
    Send-Keys "^{END}"
    Send-Keys "{ENTER}"
    Send-Keys "^v"
    Send-Keys "^s"
    $text = Wait-FileText $path { param($value) (Count-Text $value $marker) -ge 2 }
    Assert-True ((Count-Text $text $marker) -eq 2) "copy/paste should duplicate the marker once"
    Add-Step "clipboard" "copy/paste round-tripped through the system clipboard"

    $before = Get-AppRect $app
    Move-Window $app 700 500
    Start-Sleep -Milliseconds 400
    $after = Get-AppRect $app
    Assert-True ([math]::Abs(($after.Right - $after.Left) - 700) -le 24) "the window did not resize to 700 px"
    $dpi = $null
    if ([Environment]::OSVersion.Version.Build -ge 14393) {
        $dpi = [ChausSmokeNative]::GetDpiForWindow($app.MainWindowHandle)
    }
    Add-Step "resize" "resized to $($after.Right - $after.Left)x$($after.Bottom - $after.Top), dpi=$dpi"
    Save-Screenshot $app "after-resize.png"
    Close-App $app

    $vimPath = Write-Fixture "vim.txt" ("first line`nSECOND_MARKER`n")
    $vim = Start-SmokeApp "vim.txt" $vimPath
    Send-Keys "^+v"
    Start-Sleep -Milliseconds 400
    Send-Keys "gg"
    Send-Keys "dd"
    Send-Keys "^s"
    $vimText = Wait-FileText $vimPath { param($value) -not $value.Contains("first line") }
    Assert-True ($vimText.Contains("SECOND_MARKER")) "dd removed the wrong line"
    Add-Step "vim" "Ctrl+Shift+V entered Normal mode and dd removed the first line"
    Close-App $vim

    $closePath = Write-Fixture "close.txt" "base line"
    $close = Start-SmokeApp "close.txt" $closePath
    Send-Keys "^{END}"
    Send-Literal $marker
    Send-Keys "%{F4}"
    Start-Sleep -Milliseconds 1200
    $close.Refresh()
    Assert-True (-not $close.HasExited) "a dirty window closed without asking"
    Assert-True ($close.MainWindowHandle -ne [IntPtr]::Zero) "the dirty window lost its main window"
    Send-Keys "{ESC}"
    Start-Sleep -Milliseconds 500
    $close.Refresh()
    Assert-True (-not $close.HasExited) "Escape on the close sheet closed the window"
    Send-Keys "^s"
    [void](Wait-FileText $closePath { param($value) $value.Contains($marker) })
    Add-Step "dirty-close" "Alt+F4 showed the sheet, Escape cancelled, save preserved the edit"
    Close-App $close

    if (-not $SkipDialogs) {
        $openPath = Write-Fixture "open-me.txt" "dialog smoke"
        $dialog = Start-SmokeApp "Untitled"
        Send-Keys "^o"
        Start-Sleep -Milliseconds 1200
        $foreground = [ChausSmokeNative]::GetForegroundWindow()
        $foregroundPid = [uint32]0
        [void][ChausSmokeNative]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid)
        Assert-True ($foregroundPid -eq $dialog.Id) "the Open dialog did not take foreground"
        Send-Literal $openPath
        Send-Keys "{ENTER}"
        Wait-AppWindow $dialog "open-me.txt" 15
        Add-Step "open-dialog" "typed a full path into the Win32 Open dialog"

        $saveAs = Join-Path $script:WorkDir "saved-as.txt"
        Send-Keys "^+s"
        Start-Sleep -Milliseconds 1200
        Send-Literal $saveAs
        Send-Keys "{ENTER}"
        $deadline = (Get-Date).AddSeconds(10)
        while (-not (Test-Path $saveAs) -and (Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 150
        }
        Assert-True (Test-Path $saveAs) "Save As did not create $saveAs"
        Add-Step "save-as-dialog" "typed a full path into the Win32 Save As dialog"
        Close-App $dialog
    }
}

$failure = $null
try {
    Run-Smoke
} catch {
    $failure = $_.Exception.Message
    Write-Host "[fail] $failure" -ForegroundColor Red
    if ($Screenshots -and $script:Processes.Count -gt 0) {
        try { Save-Screenshot $script:Processes[-1] "failure.png" } catch { }
    }
} finally {
    foreach ($process in $script:Processes) {
        try {
            if (-not $process.HasExited) { $process.Kill(); [void]$process.WaitForExit(5000) }
        } catch { }
    }

    New-SmokeDir $script:ReportDir
    $exePath = (Resolve-Path $Exe).Path
    $report = [ordered]@{
        platform = "windows"
        started = $script:Started
        finished = (Get-Date).ToUniversalTime().ToString("o")
        executable = $exePath
        sha256 = (Get-FileHash -Algorithm SHA256 -Path $exePath).Hash.ToLowerInvariant()
        os = [Environment]::OSVersion.VersionString
        steps = $script:Steps
        failure = $failure
        skipped_dialogs = [bool]$SkipDialogs
        root = $script:Root
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 (Join-Path $script:ReportDir "smoke-report.json")
    Write-Host "report: $(Join-Path $script:ReportDir 'smoke-report.json')"
}

if ($failure) { exit 1 }
exit 0
