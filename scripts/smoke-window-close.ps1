# Run on an interactive Windows desktop with only the Luma build under test running.
param([string]$Executable, [switch]$Repeat)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class LumaWindowTest {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(long point);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr wParam, IntPtr lParam, uint flags, uint timeout, out IntPtr result);
}
'@
[void][LumaWindowTest]::SetProcessDPIAware()
$shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Luma.lnk'
if ($Executable) {
    $exe = (Resolve-Path -LiteralPath $Executable).Path
    $launcher = $exe
} else {
    $null = Get-Item -LiteralPath $shortcut
    $shell = New-Object -ComObject WScript.Shell
    $exe = $shell.CreateShortcut($shortcut).TargetPath
    $launcher = $shortcut
}
function Wait-MainWindow {
    for ($i = 0; $i -lt 40; $i++) {
        $candidate = Get-Process -Name snaptrans -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe -and $_.MainWindowTitle -like 'Luma -*' } | Select-Object -First 1
        if ($candidate -and [LumaWindowTest]::IsWindowVisible($candidate.MainWindowHandle)) { return $candidate }
        Start-Sleep -Milliseconds 150
    }
    throw 'Luma main window did not appear.'
}
function Open-Luma {
    $running = Get-Process -Name snaptrans -ErrorAction SilentlyContinue | Where-Object Path -eq $exe
    Start-Process -FilePath $launcher
    if (-not $running) {
        # A cold launch can start in the tray. Open Settings only once the first process is up.
        Start-Sleep -Milliseconds 2200
        if (-not (Get-Process -Name snaptrans -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe -and $_.MainWindowTitle -like 'Luma -*' })) {
            Start-Process -FilePath $launcher
        }
    }
    return Wait-MainWindow
}
function Test-Close($process, [switch]$Keyboard) {
    $hwnd = $process.MainWindowHandle
    [void][LumaWindowTest]::SetForegroundWindow($hwnd)
    if ([LumaWindowTest]::GetForegroundWindow() -ne $hwnd) { throw 'Luma is not the foreground window.' }
    $rect = New-Object LumaWindowTest+Rect
    if (-not [LumaWindowTest]::GetWindowRect($hwnd, [ref]$rect)) { throw 'Could not read window bounds.' }
    if ($Keyboard) {
        [LumaWindowTest]::keybd_event(0x12,0x38,0,[UIntPtr]::Zero)
        [LumaWindowTest]::keybd_event(0x73,0x3e,0,[UIntPtr]::Zero)
        [LumaWindowTest]::keybd_event(0x73,0x3e,2,[UIntPtr]::Zero)
        [LumaWindowTest]::keybd_event(0x12,0x38,2,[UIntPtr]::Zero)
    } else {
        $x = $rect.Right - 28
        $y = $rect.Top + 24
        $point = ([long]$y -shl 32) -bor ([long]$x -band 0xffffffffL)
        if ([LumaWindowTest]::GetAncestor([LumaWindowTest]::WindowFromPoint($point), 2) -ne $hwnd) { throw 'The close button is covered by another window.' }
        $position = [IntPtr]((($y -band 0xffff) -shl 16) -bor ($x -band 0xffff))
        $hit = [IntPtr]::Zero
        $ok = [LumaWindowTest]::SendMessageTimeout($hwnd, 0x84, [IntPtr]::Zero, $position, 2, 700, [ref]$hit)
        if ($ok -eq [IntPtr]::Zero -or $hit.ToInt64() -ne 20) { throw "Expected native close button at $x,$y, got $hit." }
        # Re-enter the caption after re-opening, so DWM initializes the hovered button.
        [void][LumaWindowTest]::SetCursorPos(($rect.Left + 150), ($rect.Top + 70))
        Start-Sleep -Milliseconds 120
        [void][LumaWindowTest]::SetCursorPos($x, $y)
        Start-Sleep -Milliseconds 150
        [LumaWindowTest]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
        Start-Sleep -Milliseconds 80
        [LumaWindowTest]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    }
    Start-Sleep -Milliseconds 800
    $visible = [LumaWindowTest]::IsWindowVisible($hwnd)
    $reply = [IntPtr]::Zero
    $responsive = [LumaWindowTest]::SendMessageTimeout($hwnd, 0, [IntPtr]::Zero, [IntPtr]::Zero, 2, 700, [ref]$reply) -ne [IntPtr]::Zero
    $action = if ($Keyboard) { 'Alt+F4' } else { 'Native close button' }
    Write-Output "$action | HWND=$hwnd | visible=$visible | responsive=$responsive"
    if ($visible -or -not $responsive) { throw "$action failed to close the main window cleanly." }
}

$main = Open-Luma
Test-Close $main
if ($Repeat) {
    $reopened = Open-Luma
    if ($reopened.Id -ne $main.Id -or $reopened.MainWindowHandle -ne $main.MainWindowHandle) { throw 'Reopening did not restore the same application window.' }
    Test-Close $reopened -Keyboard
    $reopened = Open-Luma
    Test-Close $reopened
    $reopened = Open-Luma
    Write-Output "Luma reopened and remains open (PID $($reopened.Id))."
}
