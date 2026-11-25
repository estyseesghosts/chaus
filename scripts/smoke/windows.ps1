<#
.SYNOPSIS
    Native Windows smoke test for a packaged chaus executable.

.DESCRIPTION
    Copies the executable and fixtures into a temporary work directory and a
    temporary profile, launches the packaged copy, and drives it with real
    keyboard and mouse input. It proves window creation, focus, typing,
    clipboard, save, resize, Vim toggle and the dirty-close sheet.

    The file-panel phase uses the real Silvestris OpenPanel and SavePanel at
    the pinned Chaus revision. Open is an application-modal browser window;
    Save As is a sheet with a Name field that resolves relative to the
    panel's current location. No Win32 file dialog is involved and no panel
    internals are called. The fixture document is opened from the temporary
    work directory so both panels start at a deterministic location.

    This script controls the real desktop. Run it on a dedicated machine or
    an interactive runner, never while using the computer.

.PARAMETER Exe
    Path to chaus.exe. It is copied before launch; the checkout is not used
    as the working directory.

.PARAMETER ReportDir
    Directory for smoke-report.json. Defaults to a new directory under TEMP.

.PARAMETER SkipPanels
    Skip the OpenPanel/SavePanel phase for machines without a usable desktop.
    Alias: SkipDialogs (the old name from the Win32-dialog script).

.PARAMETER Screenshots
    Save PNG screenshots at the end of each phase. They are evidence only and
    never gate the result.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Exe,

    [string]$ReportDir = "",

    [Alias('SkipDialogs')]
    [switch]$SkipPanels,

    [switch]$Screenshots
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class ChausSmokeNative {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int max);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
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

function Set-ClipboardText([string]$Text) {
    [System.Windows.Forms.Clipboard]::SetText($Text)
    Start-Sleep -Milliseconds 150
}

function Paste-Literal([string]$Text) {
    Set-ClipboardText $Text
    Send-Keys "^a"
    Send-Keys "^v"
}

function Get-ForegroundTitle() {
    $handle = [ChausSmokeNative]::GetForegroundWindow()
    $builder = New-Object System.Text.StringBuilder 512
    [void][ChausSmokeNative]::GetWindowText($handle, $builder, 512)
    return $builder.ToString()
}

function Get-AppWindows([int]$ProcessId) {
    $windows = New-Object System.Collections.ArrayList
    $callback = [ChausSmokeNative+EnumWindowsProc]{
        param($handle, $unused)
        if (-not [ChausSmokeNative]::IsWindowVisible($handle)) { return $true }
        $owner = [uint32]0
        [void][ChausSmokeNative]::GetWindowThreadProcessId($handle, [ref]$owner)
        if ($owner -ne $ProcessId) { return $true }
        $builder = New-Object System.Text.StringBuilder 512
        [void][ChausSmokeNative]::GetWindowText($handle, $builder, 512)
        if ($builder.Length -gt 0) {
            [void]$windows.Add([pscustomobject]@{
                handle = $handle
                title = $builder.ToString()
            })
        }
        return $true
    }
    [void][ChausSmokeNative]::EnumWindows($callback, [IntPtr]::Zero)
    return ,$windows.ToArray()
}

function Get-AppWindowTitles([int]$ProcessId) {
    return @(Get-AppWindows $ProcessId | ForEach-Object { $_.title })
}

function Wait-WindowTitle(
    [System.Diagnostics.Process]$Process,
    [string]$Title,
    [int]$Seconds = 15
) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if (Get-AppWindowTitles $Process.Id -contains $Title) { return }
        Start-Sleep -Milliseconds 150
    }
    $titles = Get-AppWindowTitles $Process.Id
    throw "the window titled '$Title' did not appear (windows: $($titles -join ', '))"
}

function Wait-WindowGone(
    [System.Diagnostics.Process]$Process,
    [string]$Title,
    [int]$Seconds = 15
) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if (Get-AppWindowTitles $Process.Id -notcontains $Title) { return }
        Start-Sleep -Milliseconds 150
    }
    throw "the window titled '$Title' did not close"
}

function Assert-ForegroundOwner(
    [System.Diagnostics.Process]$Process,
    [string]$ExpectedTitle = ""
) {
    $title = Get-ForegroundTitle
    $window = Get-AppWindows $Process.Id | Where-Object { $_.title -eq $title } | Select-Object -First 1
    Assert-True ($null -ne $window) "foreground window '$title' does not belong to the app"
    if ($ExpectedTitle) {
        Assert-True ($title -eq $ExpectedTitle) "expected '$ExpectedTitle' in the foreground, got '$title'"
    }
}

function Focus-AppWindow(
    [System.Diagnostics.Process]$Process,
    [string]$Title
) {
    $window = Get-AppWindows $Process.Id | Where-Object { $_.title -eq $Title } | Select-Object -First 1
    Assert-True ($null -ne $window) "cannot focus missing window '$Title'"
    [void][ChausSmokeNative]::ShowWindow($window.handle, 9)
    [void][ChausSmokeNative]::SetForegroundWindow($window.handle)
    Start-Sleep -Milliseconds 400
    $rect = New-Object ChausSmokeNative+RECT
    Assert-True ([ChausSmokeNative]::GetWindowRect($window.handle, [ref]$rect)) "GetWindowRect failed"
    [void][ChausSmokeNative]::SetCursorPos(
        [int](($rect.Left + $rect.Right) / 2),
        [int](($rect.Top + $rect.Bottom) / 2)
    )
    [ChausSmokeNative]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    [ChausSmokeNative]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 400
    Assert-ForegroundOwner $Process $Title
}

function Wait-PanelForeground(
    [System.Diagnostics.Process]$Process,
    [string]$Title
) {
    $deadline = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $deadline) {
        if ((Get-ForegroundTitle) -eq $Title) {
            Assert-ForegroundOwner $Process $Title
            return
        }
        Start-Sleep -Milliseconds 150
    }
    throw "the Silvestris panel '$Title' did not take the foreground (got '$(Get-ForegroundTitle)')"
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

    Assert-ForegroundOwner $Process
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
    return Get-Content -Raw -LiteralPath (Join-Path $script:WorkDir $Name)
}

function Write-Fixture([string]$Name, [string]$Text) {
    $path = Join-Path $script:WorkDir $Name
    [System.IO.File]::WriteAllText($path, $Text)
    return $path
}

function Wait-FileText([string]$Path, [scriptblock]$Predicate, [int]$Seconds = 10) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path -LiteralPath $Path) {
            $text = Get-Content -Raw -LiteralPath $Path
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

function Open-SaveSheet([string]$Name, [bool]$UseClipboard = $false) {
    Send-Keys "^+s"
    Start-Sleep -Milliseconds 1200
    if ($UseClipboard) {
        Paste-Literal $Name
    } else {
        Send-Literal $Name
    }
}

function Invoke-SaveSheet([string]$Name, [bool]$UseClipboard = $false) {
    Open-SaveSheet $Name $UseClipboard
    Send-Keys "{ENTER}"
}

function Run-PanelPhases() {
    # The named document makes both panels start in the temporary work
    # directory instead of the provider's home or the checkout.
    $baseName = "panel-base.txt"
    $basePath = Write-Fixture $baseName "panel starting document"
    $openName = "open-me.txt"
    $openPath = Write-Fixture $openName "opened through the browser"
    $panelApp = Start-SmokeApp $baseName $basePath

    # Open: keyboard acceptance through the real browser.
    Send-Keys "^o"
    Wait-PanelForeground $panelApp "Chaus: Open"
    Add-Step "open-panel" "Ctrl+O presented the Silvestris browser at the work directory"
    Send-Keys "{TAB}{TAB}" # sidebar, then the file browser
    Start-Sleep -Milliseconds 300
    Send-Literal $openName # type-select in the browser
    Start-Sleep -Milliseconds 300
    Send-Keys "{ENTER}"
    Wait-WindowTitle $panelApp $openName 15
    Assert-ForegroundOwner $panelApp $openName
    Assert-True ((Get-Content -Raw -LiteralPath $openPath) -eq "opened through the browser") `
        "the accepted document was modified"
    Add-Step "open-accept" "selected $openName in the browser and accepted with Return"
    Assert-True ((Get-AppWindowTitles $panelApp.Id) -contains $baseName) `
        "opening a document replaced the original editor window"

    # Open: Escape cancels without creating a window and restores focus.
    Focus-AppWindow $panelApp $openName
    Send-Keys "^o"
    Wait-PanelForeground $panelApp "Chaus: Open"
    Send-Keys "{ESC}"
    Wait-WindowGone $panelApp "Chaus: Open" 8
    Assert-ForegroundOwner $panelApp $openName
    Add-Step "open-cancel" "Escape dismissed the browser, opened nothing and restored editor focus"

    # Save As: only the target filename is typed; the panel resolves it
    # relative to the current location.
    $saveName = "saved-as.txt"
    $savePath = Join-Path $script:WorkDir $saveName
    Invoke-SaveSheet $saveName
    Wait-FileText $savePath { param($value) $value.Contains("opened through the browser") }
    Wait-WindowTitle $panelApp $saveName 15
    Add-Step "save-as-accept" "typed only $saveName in the Name field; the panel resolved it in the work directory"

    # Save As: Escape writes nothing and restores focus.
    $cancelName = "cancel-as.txt"
    Open-SaveSheet $cancelName
    Send-Keys "{ESC}"
    Start-Sleep -Milliseconds 700
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $script:WorkDir $cancelName))) `
        "a cancelled Save As still wrote a file"
    Assert-ForegroundOwner $panelApp $saveName
    Add-Step "save-as-cancel" "Escape dismissed the Save sheet without writing"

    # Replace existing: the panel asks first; Return takes the default
    # Replace button and the document becomes associated with the target.
    $replaceName = "replace-target.txt"
    $replacePath = Write-Fixture $replaceName "old contents"
    Invoke-SaveSheet $replaceName
    Wait-WindowTitle $panelApp "Replace" 15
    Assert-ForegroundOwner $panelApp "Replace"
    Send-Keys "{ENTER}"
    Wait-FileText $replacePath { param($value) $value.Contains("opened through the browser") }
    Wait-WindowTitle $panelApp $replaceName 15
    Add-Step "replace-existing" "Replace confirmed the collision and wrote the existing target"

    # Replace cancel returns to the sheet; a second Escape leaves the target
    # and the document association alone.
    $keepName = "keep-target.txt"
    $keepPath = Write-Fixture $keepName "keep me"
    Invoke-SaveSheet $keepName
    Wait-WindowTitle $panelApp "Replace" 15
    Send-Keys "{ESC}"
    Start-Sleep -Milliseconds 500
    Send-Keys "{ESC}"
    Start-Sleep -Milliseconds 700
    Assert-True ((Get-Content -Raw -LiteralPath $keepPath) -eq "keep me") `
        "cancelling Replace overwrote the existing target"
    Assert-ForegroundOwner $panelApp $replaceName
    Add-Step "replace-cancel" "Escape on the alert returned to the sheet; Escape there kept the target"

    # Unicode filename: paste avoids layout-dependent SendKeys behaviour but
    # still goes through the real Name field and the real browser provider.
    $unicodeName = "n" + [char]0x00F6 + "te-" + [char]0x6587 + [char]0x4EF6 + ".txt"
    $unicodePath = Join-Path $script:WorkDir $unicodeName
    Invoke-SaveSheet $unicodeName $true
    Wait-FileText $unicodePath { param($value) $value.Contains("opened through the browser") }
    Wait-WindowTitle $panelApp $unicodeName 15
    Add-Step "unicode-save-as" "a pasted Unicode name round-tripped through the Name field"

    # Dirty close on an untitled window routes Save through SavePanel.
    Send-Keys "^n"
    Start-Sleep -Milliseconds 800
    Send-Literal "DIRTYCLOSE"
    Send-Keys "%{F4}"
    Start-Sleep -Milliseconds 1200
    $untitledSaved = "dirty-close-save.txt"
    $untitledPath = Join-Path $script:WorkDir $untitledSaved
    Assert-True (-not $panelApp.HasExited) "the dirty window closed without asking"
    Send-Keys "{ENTER}" # the close sheet focuses its Save button
    Start-Sleep -Milliseconds 1200
    Paste-Literal $untitledSaved
    Send-Keys "{ENTER}"
    Wait-FileText $untitledPath { param($value) $value.Contains("DIRTYCLOSE") }
    Add-Step "dirty-close-save-as" "dirty close -> Save used the real Save sheet for an untitled document"
    Assert-ForegroundOwner $panelApp $unicodeName

    Close-App $panelApp
}

function Run-BasicPhases() {
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
}

function Run-Smoke() {
    New-SmokeDir $script:Root
    New-SmokeDir $script:AppDir
    New-SmokeDir $script:WorkDir
    New-SmokeDir $script:ReportDir
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $script:AppDir "chaus.exe") -Force
    Assert-True (Test-Path -LiteralPath (Join-Path $script:AppDir "chaus.exe")) "the executable was not copied"

    Run-BasicPhases
    if (-not $SkipPanels) { Run-PanelPhases }
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
        panels = "Silvestris OpenPanel/SavePanel (pinned f1c2a010)"
        steps = $script:Steps
        failure = $failure
        skipped_panels = [bool]$SkipPanels
        root = $script:Root
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 (Join-Path $script:ReportDir "smoke-report.json")
    Write-Host "report: $(Join-Path $script:ReportDir 'smoke-report.json')"
}

if ($failure) { exit 1 }
exit 0
