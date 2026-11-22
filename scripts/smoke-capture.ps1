param([Parameter(Mandatory)][string]$Executable)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class CaptureTest {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
  private delegate bool EnumProc(IntPtr hwnd, IntPtr unused);
  [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr value);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
  [DllImport("user32.dll")] private static extern int GetWindowRgn(IntPtr hwnd, IntPtr hrgn);
  [DllImport("gdi32.dll")] private static extern IntPtr CreateRectRgn(int x1, int y1, int x2, int y2);
  [DllImport("gdi32.dll")] private static extern int GetRgnBox(IntPtr hrgn, out Rect rect);
  [DllImport("gdi32.dll")] private static extern bool DeleteObject(IntPtr handle);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int chars);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
  [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] private static extern IntPtr GetDC(IntPtr hwnd);
  [DllImport("user32.dll")] private static extern int ReleaseDC(IntPtr hwnd, IntPtr dc);
  [DllImport("gdi32.dll")] private static extern uint GetPixel(IntPtr dc, int x, int y);
  public static uint ScreenPixel(int x, int y) {
    IntPtr dc = GetDC(IntPtr.Zero);
    try { return GetPixel(dc, x, y); } finally { ReleaseDC(IntPtr.Zero, dc); }
  }
  public static bool GetVisibleBounds(IntPtr hwnd, out Rect rect) {
    if (!GetWindowRect(hwnd, out rect)) return false;
    IntPtr rgn = CreateRectRgn(0, 0, 0, 0);
    if (rgn == IntPtr.Zero) return true;
    try {
      if (GetWindowRgn(hwnd, rgn) == 0) return true;
      Rect box;
      if (GetRgnBox(rgn, out box) == 0) return true;
      rect = new Rect {
        Left = rect.Left + box.Left,
        Top = rect.Top + box.Top,
        Right = rect.Left + box.Right,
        Bottom = rect.Top + box.Bottom
      };
      return true;
    } finally {
      DeleteObject(rgn);
    }
  }
  public static IntPtr Find(uint expectedPid, string titlePrefix) {
    IntPtr result = IntPtr.Zero;
    EnumWindows((hwnd, unused) => {
      uint pid;
      GetWindowThreadProcessId(hwnd, out pid);
      if (pid != expectedPid) return true;
      var title = new StringBuilder(128);
      GetWindowText(hwnd, title, title.Capacity);
      if (!title.ToString().StartsWith(titlePrefix, StringComparison.Ordinal)) return true;
      result = hwnd;
      return false;
    }, IntPtr.Zero);
    return result;
  }
}
'@
[void][CaptureTest]::SetProcessDPIAware()
$exe = (Resolve-Path -LiteralPath $Executable).Path
$p = Get-Process -Name snaptrans | Where-Object Path -eq $exe | Select-Object -First 1
if (-not $p) { throw 'Start the Luma executable under test first.' }
$configDir = Join-Path $env:APPDATA 'com.snaptrans.app'
$settings = Get-Content -Raw -LiteralPath (Join-Path $configDir 'config.json') | ConvertFrom-Json
if ($settings.shortcut -notmatch '^Alt\+([A-Z])$') { throw 'This local test needs an Alt+letter capture shortcut.' }
$hotkey = [byte][char]$Matches[1]
function Send-Key([byte]$key) {
    [CaptureTest]::keybd_event($key, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 35
    [CaptureTest]::keybd_event($key, 0, 2, [UIntPtr]::Zero)
}
function Send-Hotkey {
    [CaptureTest]::keybd_event(0x12, 0x38, 0, [UIntPtr]::Zero)
    Send-Key $hotkey
    [CaptureTest]::keybd_event(0x12, 0x38, 2, [UIntPtr]::Zero)
}
function Wait-State([IntPtr]$hwnd, [bool]$visible, [string]$label) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do {
        if ([CaptureTest]::IsWindowVisible($hwnd) -eq $visible) { return }
        Start-Sleep -Milliseconds 25
    } while ($watch.ElapsedMilliseconds -lt 6500)
    throw "Timeout waiting for $label visible=$visible."
}
function Get-WindowBounds([IntPtr]$hwnd) {
    $rect = New-Object CaptureTest+Rect
    if (-not [CaptureTest]::GetVisibleBounds($hwnd, [ref]$rect)) { throw 'Window bounds unavailable.' }
    return $rect
}
function Drag-Box([int]$x,[int]$y,[int]$w,[int]$h) {
    [void][CaptureTest]::SetCursorPos($x,$y)
    [CaptureTest]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 75
    [void][CaptureTest]::SetCursorPos(($x+$w),($y+$h))
    Start-Sleep -Milliseconds 110
    [CaptureTest]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 200
}
function Click-At([int]$x,[int]$y) {
    [void][CaptureTest]::SetCursorPos($x,$y)
    Start-Sleep -Milliseconds 80
    [CaptureTest]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [CaptureTest]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
}
$startup = [Diagnostics.Stopwatch]::StartNew()
do {
    $overlay = [CaptureTest]::Find($p.Id, 'Luma Capture')
    $stack = [CaptureTest]::Find($p.Id, 'Luma Stack')
    if ($overlay -ne [IntPtr]::Zero -and $stack -ne [IntPtr]::Zero) { break }
    Start-Sleep -Milliseconds 100
} while ($startup.ElapsedMilliseconds -lt 6500)
if ($overlay -eq [IntPtr]::Zero -or $stack -eq [IntPtr]::Zero) { throw 'Capture or Stack window not found.' }
Wait-State $overlay $false 'Capture initially hidden'
Wait-State $stack $false 'Empty Stack initially hidden'

# A static window supplies known colors and synthetic, non-personal capture content.
$bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds
$form = New-Object Windows.Forms.Form
$form.Text = 'Luma smoke-test image'
$form.FormBorderStyle = 'None'
$form.StartPosition = 'Manual'
$form.Bounds = New-Object Drawing.Rectangle(($bounds.Left+120),($bounds.Top+140),720,340)
$form.BackColor = [Drawing.Color]::FromArgb(28,106,178)
$form.Add_Paint({param($sender,$eventArgs) $eventArgs.Graphics.FillRectangle([Drawing.Brushes]::Chocolate,400,0,320,340)})
$form.Show()
$form.Activate()
[Windows.Forms.Application]::DoEvents()
$sampleX = $form.Left + 80
$sampleY = $form.Top + 100
[void][CaptureTest]::SetCursorPos(($form.Left+150),($form.Top+150))
try {
    $expected = [CaptureTest]::ScreenPixel($sampleX,$sampleY)
    Send-Hotkey
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $mismatchedFrames = 0
    do {
        if ([CaptureTest]::ScreenPixel($sampleX,$sampleY) -ne $expected) { $mismatchedFrames++ }
        if ($timer.ElapsedMilliseconds -gt 6500) { throw 'The capture overlay did not present.' }
        Start-Sleep -Milliseconds 8
    } while (-not [CaptureTest]::IsWindowVisible($overlay))
    $presentMs = $timer.ElapsedMilliseconds
    if ([CaptureTest]::GetForegroundWindow() -ne $overlay) {
        Start-Sleep -Milliseconds 150
        if ([CaptureTest]::GetForegroundWindow() -ne $overlay) { throw 'Capture did not receive keyboard focus.' }
    }
    Send-Hotkey
    $timer.Restart()
    do {
        if (-not [CaptureTest]::IsWindowVisible($overlay)) { throw 'A repeated hotkey hid the active capture.' }
        if ([CaptureTest]::ScreenPixel($sampleX,$sampleY) -ne $expected) { $mismatchedFrames++ }
        Start-Sleep -Milliseconds 8
    } while ($timer.ElapsedMilliseconds -lt 700)
    if ($mismatchedFrames -ne 0) { throw "Capture transition changed the reference pixel in $mismatchedFrames samples." }
    Write-Output "Hotkey presented the overlay in $presentMs ms. Reference pixels matched and the repeated hotkey kept it visible."
    Send-Key 0x1B
    Wait-State $overlay $false 'Capture cancelled by Escape'

    Send-Hotkey
    Wait-State $overlay $true 'Capture reopened for Save'
    Start-Sleep -Milliseconds 140
    Drag-Box ($form.Left+30) ($form.Top+30) 320 105
    Send-Key 0x53 # S: open the Save dialog
    Wait-State $overlay $false 'Capture hidden behind Save dialog'
    Start-Sleep -Milliseconds 4200
    Send-Key 0x1B
    Wait-State $overlay $true 'Capture restored after cancelling Save'
    Send-Key 0x1B
    Wait-State $overlay $false 'Restored capture cancelled'
    Write-Output 'Cancelling Save restored the selection after the initial overlay timeout.'

    foreach ($offset in @(0,140)) {
        Send-Hotkey
        Wait-State $overlay $true 'Capture reopened'
        Start-Sleep -Milliseconds 140
        Drag-Box ($form.Left+30+$offset) ($form.Top+30) (320+$offset) 105
        Send-Key 0x42         # B: stage the selected region
        Wait-State $overlay $false 'Capture staged'
        Wait-State $stack $true 'Collapsed Stack'
    }
    Start-Sleep -Milliseconds 400
    $deck = Get-WindowBounds $stack
    $deckW = $deck.Right-$deck.Left
    $deckH = $deck.Bottom-$deck.Top
    Click-At ($deck.Left+[int]($deckW/2)) ($deck.Top+[int]($deckH/2))
    Start-Sleep -Milliseconds 750
    $preview = Get-WindowBounds $stack
    if (($preview.Right-$preview.Left) -lt 2*$deckW) { throw 'Stack click did not open 3D preview.' }
    Send-Key 0x25 # Left arrow: previous card
    Start-Sleep -Milliseconds 550
    Send-Key 0x1B
    Start-Sleep -Milliseconds 500
    $collapsedAgain = Get-WindowBounds $stack
    if (($collapsedAgain.Right-$collapsedAgain.Left) -ne $deckW) { throw 'Escape did not collapse the 3D preview.' }
    Click-At ($collapsedAgain.Right-[int]($deckW*0.16)) ($collapsedAgain.Bottom-[int]($deckH*0.13))
    Start-Sleep -Milliseconds 650
    $workspace = Get-WindowBounds $stack
    if (($workspace.Right-$workspace.Left) -le $deckW -or ($workspace.Right-$workspace.Left) -ge ($preview.Right-$preview.Left)) { throw 'Ask AI did not open the Stack workspace.' }
    Write-Output 'Two captures staged successfully. The 3D preview, Escape return, and Ask AI workspace all opened.'
    Send-Key 0x1B
    Start-Sleep -Milliseconds 300
    Send-Hotkey
    Wait-State $overlay $true 'Capture for Pin'
    Start-Sleep -Milliseconds 160
    Drag-Box ($form.Left+40) ($form.Top+40) 370 48
    Send-Key 0x50 # P: pin the selected region
    Wait-State $overlay $false 'Capture pinned'
    Start-Sleep -Milliseconds 500
    $pin = [CaptureTest]::Find($p.Id, 'Luma '+[char]0x8D34+[char]0x56FE)
    if ($pin -eq [IntPtr]::Zero -or -not [CaptureTest]::IsWindowVisible($pin)) { throw 'Pin not visible.' }
    $pinRect = Get-WindowBounds $pin
    $pinWidth = $pinRect.Right-$pinRect.Left
    $pinHeight = $pinRect.Bottom-$pinRect.Top
    if ($pinWidth -lt 4*$pinHeight) { throw 'A short wide image created a tall pin window.' }
    Click-At ($pinRect.Right-28) ($pinRect.Top+30)
    Wait-State $pin $false 'Pin closed with corner button'
    Write-Output "Short pin ${pinWidth}x${pinHeight} closed with its visible corner button."
} finally { $form.Dispose() }
