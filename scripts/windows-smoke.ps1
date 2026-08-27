<#
.SYNOPSIS
    Clean-machine smoke test for the Windows build.

.DESCRIPTION
    winget's validation VM is a *clean* Windows image, and it rejected Ziplark
    twice (microsoft/winget-pkgs#395332) because ziplark-gui.exe died with
    STATUS_DLL_NOT_FOUND (0xC0000135 / -1073741515) before main() ran: our C
    deps pulled in VCRUNTIME140.dll and libunrar (C++) pulled in MSVCP140.dll,
    neither of which ships with Windows.

    CI runners are NOT clean machines -- they have the VC++ redistributable
    installed -- so simply running the binary on a runner proves nothing. This
    script therefore *statically* proves the property instead: it parses each
    PE import table (plus the delay-load table) and asserts that every DLL our
    binaries reference is one that ships with Windows itself. If that holds,
    STATUS_DLL_NOT_FOUND cannot happen, on any machine.

    It then also exercises the binaries end to end: a CLI create/list/extract
    roundtrip, an MCP stdio JSON-RPC handshake, and a GUI launch.

.EXAMPLE
    pwsh scripts/windows-smoke.ps1
    pwsh scripts/windows-smoke.ps1 -BinDir target/x86_64-pc-windows-msvc/release
#>
[CmdletBinding()]
param(
    [string]$BinDir = "target/release",
    # Skip the windowed launch (useful in a session with no desktop).
    [switch]$SkipGui
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:Failures = @()
function Fail([string]$m) { $script:Failures += $m; Write-Host "  FAIL  $m" -ForegroundColor Red }
function Pass([string]$m) { Write-Host "  ok    $m" -ForegroundColor Green }
function Section([string]$m) { Write-Host ""; Write-Host "== $m" -ForegroundColor Cyan }

# ---------------------------------------------------------------------------
# PE import-table reader (no external tooling -- no dumpbin, no VS required)
# ---------------------------------------------------------------------------
function Get-PeImportedDll {
    param([Parameter(Mandatory)][string]$Path)

    $b = [System.IO.File]::ReadAllBytes($Path)
    if ($b.Length -lt 0x40 -or $b[0] -ne 0x4D -or $b[1] -ne 0x5A) { throw "not a PE image: $Path" }

    $peOff = [BitConverter]::ToUInt32($b, 0x3C)
    if ([BitConverter]::ToUInt32($b, $peOff) -ne 0x00004550) { throw "bad PE signature: $Path" }

    $coff         = $peOff + 4
    $numSections  = [BitConverter]::ToUInt16($b, $coff + 2)
    $sizeOfOptHdr = [BitConverter]::ToUInt16($b, $coff + 16)
    $opt          = $coff + 20
    $magic        = [BitConverter]::ToUInt16($b, $opt)

    # Data directories sit right after NumberOfRvaAndSizes.
    $ddOff = switch ($magic) {
        0x10B   { $opt + 96 }   # PE32
        0x20B   { $opt + 112 }  # PE32+
        default { throw ("unknown optional header magic 0x{0:X}: {1}" -f $magic, $Path) }
    }

    $secOff = $opt + $sizeOfOptHdr
    $sections = for ($i = 0; $i -lt $numSections; $i++) {
        $s = $secOff + ($i * 40)
        [pscustomobject]@{
            VirtualAddress   = [BitConverter]::ToUInt32($b, $s + 12)
            VirtualSize      = [BitConverter]::ToUInt32($b, $s + 8)
            SizeOfRawData    = [BitConverter]::ToUInt32($b, $s + 16)
            PointerToRawData = [BitConverter]::ToUInt32($b, $s + 20)
        }
    }

    $rvaToOff = {
        param([uint32]$rva)
        foreach ($s in $sections) {
            $span = [Math]::Max($s.VirtualSize, $s.SizeOfRawData)
            if ($rva -ge $s.VirtualAddress -and $rva -lt ($s.VirtualAddress + $span)) {
                return [int]($rva - $s.VirtualAddress + $s.PointerToRawData)
            }
        }
        return 0
    }

    $readCStr = {
        param([int]$off)
        if ($off -le 0 -or $off -ge $b.Length) { return "" }
        $end = $off
        while ($end -lt $b.Length -and $b[$end] -ne 0) { $end++ }
        [System.Text.Encoding]::ASCII.GetString($b, $off, $end - $off)
    }

    $names = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)

    # Directory 1: IMAGE_IMPORT_DESCRIPTOR[] -- 20 bytes each, name RVA at +12.
    $impRva = [BitConverter]::ToUInt32($b, $ddOff + 8)
    if ($impRva -ne 0) {
        $p = & $rvaToOff $impRva
        while ($p -gt 0 -and ($p + 20) -le $b.Length) {
            $origThunk = [BitConverter]::ToUInt32($b, $p)
            $nameRva   = [BitConverter]::ToUInt32($b, $p + 12)
            $firstNext = [BitConverter]::ToUInt32($b, $p + 16)
            if ($origThunk -eq 0 -and $nameRva -eq 0 -and $firstNext -eq 0) { break }
            $n = & $readCStr (& $rvaToOff $nameRva)
            if ($n) { [void]$names.Add($n) }
            $p += 20
        }
    }

    # Directory 13: IMAGE_DELAYLOAD_DESCRIPTOR[] -- 32 bytes each, name RVA at +4.
    $delayRva = [BitConverter]::ToUInt32($b, $ddOff + (13 * 8))
    if ($delayRva -ne 0) {
        $p = & $rvaToOff $delayRva
        while ($p -gt 0 -and ($p + 32) -le $b.Length) {
            $attrs   = [BitConverter]::ToUInt32($b, $p)
            $nameRva = [BitConverter]::ToUInt32($b, $p + 4)
            if ($attrs -eq 0 -and $nameRva -eq 0) { break }
            $n = & $readCStr (& $rvaToOff $nameRva)
            if ($n) { [void]$names.Add($n) }
            $p += 32
        }
    }

    $names | Sort-Object
}

# DLLs that are part of a base Windows install (including Server Core), so they
# are always present on the winget validation image and on a user's fresh box.
$OsDlls = @(
    'ntdll.dll','kernel32.dll','kernelbase.dll','advapi32.dll','user32.dll',
    'gdi32.dll','gdi32full.dll','gdiplus.dll','shell32.dll','shlwapi.dll',
    'shcore.dll','ole32.dll','oleaut32.dll','combase.dll','oleacc.dll',
    'comdlg32.dll','comctl32.dll','crypt32.dll','bcrypt.dll','bcryptprimitives.dll',
    'ncrypt.dll','rpcrt4.dll','ws2_32.dll','userenv.dll','secur32.dll','sspicli.dll',
    'version.dll','psapi.dll','dbghelp.dll','imm32.dll','powrprof.dll','dwmapi.dll',
    'uxtheme.dll','dnsapi.dll','iphlpapi.dll','netapi32.dll','normaliz.dll',
    'winhttp.dll','wininet.dll','urlmon.dll','propsys.dll','wintrust.dll',
    'setupapi.dll','cfgmgr32.dll','msimg32.dll','winmm.dll','wtsapi32.dll',
    'mpr.dll','authz.dll','msi.dll','winspool.drv','d3d11.dll','dxgi.dll',
    'd2d1.dll','dwrite.dll','opengl32.dll','windows.storage.dll',
    # The Universal CRT ships with Windows 10+ / Server 2016+.
    'ucrtbase.dll','msvcrt.dll'
)

# The Visual C++ *redistributable* -- NOT part of Windows. These are exactly
# what broke the winget validation run.
$RedistDlls = @(
    'vcruntime140.dll','vcruntime140_1.dll','vcruntime140d.dll',
    'msvcp140.dll','msvcp140_1.dll','msvcp140_2.dll','msvcp140_atomic_wait.dll',
    'msvcp140_codecvt_ids.dll','concrt140.dll','vcomp140.dll','vccorlib140.dll',
    'msvcr120.dll','msvcr110.dll','msvcr100.dll','msvcp120.dll',
    # Shipped by the WebView2 SDK rather than by Windows.
    'webview2loader.dll'
)

function Test-Imports {
    param([Parameter(Mandatory)][string]$Path)

    $name = Split-Path $Path -Leaf
    if (-not (Test-Path $Path)) { Fail "$name -- not built (missing $Path)"; return }

    $imports = @(Get-PeImportedDll -Path $Path)
    Write-Host "  $name imports: $($imports -join ', ')" -ForegroundColor DarkGray

    $bad = @()
    foreach ($dll in $imports) {
        $lower = $dll.ToLowerInvariant()
        if ($RedistDlls -contains $lower) {
            $bad += "$dll (Visual C++ redistributable -- absent on a clean Windows)"
            continue
        }
        # api-ms-win-* / ext-ms-win-* are OS API sets, always resolvable.
        if ($lower -like 'api-ms-win-*' -or $lower -like 'ext-ms-*') { continue }
        if ($OsDlls -contains $lower) { continue }
        $bad += "$dll (not known to ship with Windows)"
    }

    if ($bad.Count) {
        Fail "$name depends on DLLs a clean Windows does not have:`n        - $($bad -join "`n        - ")"
    } else {
        Pass "$name imports only DLLs that ship with Windows ($($imports.Count) of them)"
    }
}

# ---------------------------------------------------------------------------
$root   = Split-Path $PSScriptRoot -Parent
$bin    = Join-Path $root $BinDir
$gui    = Join-Path $bin 'ziplark-gui.exe'
$cli    = Join-Path $bin 'ziplark.exe'
$mcp    = Join-Path $bin 'ziplark-mcp.exe'

Write-Host "Ziplark Windows smoke test" -ForegroundColor White
Write-Host "binaries: $bin"

Section "1. Import tables must be satisfiable on a clean Windows install"
if (-not $SkipGui) { Test-Imports $gui }
Test-Imports $cli
Test-Imports $mcp

Section "2. CLI roundtrip"
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("ziplark-smoke-" + [Guid]::NewGuid().ToString('N'))
$src  = Join-Path $work 'src'
$out  = Join-Path $work 'out'
New-Item -ItemType Directory -Force -Path (Join-Path $src 'nested') | Out-Null
'hello from ziplark'          | Set-Content -NoNewline (Join-Path $src 'a.txt')
'nested payload'              | Set-Content -NoNewline (Join-Path $src 'nested\b.txt')
$zip = Join-Path $work 'smoke.zip'

try {
    & $cli create $zip $src | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "create exited $LASTEXITCODE" }
    if (-not (Test-Path $zip)) { throw "archive was not produced" }
    Pass "ziplark create -> $([math]::Round((Get-Item $zip).Length / 1KB, 1)) KB"

    $listing = & $cli list $zip --json
    if ($LASTEXITCODE -ne 0) { throw "list exited $LASTEXITCODE" }
    $parsed = $listing | ConvertFrom-Json
    Pass "ziplark list --json parsed ($(@($parsed.entries).Count) entries)"

    & $cli test $zip | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "test exited $LASTEXITCODE" }
    Pass "ziplark test"

    & $cli extract $zip -o $out --overwrite | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "extract exited $LASTEXITCODE" }
    $roundtripped = Get-ChildItem $out -Recurse -File
    $a = $roundtripped | Where-Object Name -eq 'a.txt' | Select-Object -First 1
    $bf = $roundtripped | Where-Object Name -eq 'b.txt' | Select-Object -First 1
    if (-not $a -or -not $bf) { throw "extracted tree is missing a.txt or b.txt" }
    if ((Get-Content -Raw $a.FullName) -ne 'hello from ziplark') { throw "a.txt content differs after roundtrip" }
    if ((Get-Content -Raw $bf.FullName) -ne 'nested payload')    { throw "b.txt content differs after roundtrip" }
    Pass "ziplark extract -- contents match byte for byte"
} catch {
    Fail "CLI roundtrip: $_"
}

Section "3. MCP stdio handshake"
try {
    $requests = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
        '{"jsonrpc":"2.0","method":"notifications/initialized"}'
        '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
        ('{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ziplark_list","arguments":{"path":' +
            (ConvertTo-Json $zip) + '}}}')
    ) -join "`n"

    $reqFile = Join-Path $work 'mcp-in.jsonl'
    $resFile = Join-Path $work 'mcp-out.jsonl'
    Set-Content -Path $reqFile -Value $requests -Encoding utf8
    $p = Start-Process -FilePath $mcp -NoNewWindow -PassThru -Wait `
            -RedirectStandardInput $reqFile -RedirectStandardOutput $resFile `
            -RedirectStandardError (Join-Path $work 'mcp-err.txt')
    if ($p.ExitCode -ne 0) { throw "ziplark-mcp exited $($p.ExitCode)" }

    $responses = @(Get-Content $resFile | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json })
    $init  = $responses | Where-Object { $_.id -eq 1 } | Select-Object -First 1
    $tools = $responses | Where-Object { $_.id -eq 2 } | Select-Object -First 1
    $call  = $responses | Where-Object { $_.id -eq 3 } | Select-Object -First 1

    if (-not $init)  { throw "no response to initialize" }
    if ($init.result.serverInfo.name -ne 'ziplark') { throw "unexpected serverInfo: $($init.result.serverInfo | ConvertTo-Json -Compress)" }
    Pass "initialize -> ziplark $($init.result.serverInfo.version), protocol $($init.result.protocolVersion)"

    if (-not $tools) { throw "no response to tools/list" }
    $toolNames = @($tools.result.tools.name)
    foreach ($expected in 'ziplark_info','ziplark_list','ziplark_test') {
        if ($toolNames -notcontains $expected) { throw "tools/list is missing $expected" }
    }
    if ($toolNames -contains 'ziplark_extract') { throw "write tool exposed without --allow-write" }
    Pass "tools/list -> $($toolNames -join ', ') (read-only by default)"

    if (-not $call) { throw "no response to tools/call" }
    if ($call.result.isError) { throw "ziplark_list reported an error: $($call.result.content[0].text)" }
    if ($call.result.content[0].text -notmatch 'a\.txt') { throw "ziplark_list did not mention a.txt" }
    Pass "tools/call ziplark_list -- listed the archive"
} catch {
    Fail "MCP handshake: $_"
}

Section "4. GUI launches"
if ($SkipGui) {
    Write-Host "  skipped (-SkipGui)" -ForegroundColor Yellow
} elseif (-not (Test-Path $gui)) {
    Fail "ziplark-gui.exe not built"
} else {
    try {
        $proc = Start-Process -FilePath $gui -PassThru
        Start-Sleep -Seconds 10
        $proc.Refresh()
        if ($proc.HasExited) {
            $code = $proc.ExitCode
            if ($code -eq -1073741515) {
                Fail "ziplark-gui.exe died with STATUS_DLL_NOT_FOUND (-1073741515) -- the winget failure is NOT fixed"
            } else {
                Fail "ziplark-gui.exe exited early with code $code (0x{0:X8})" -f $code
            }
        } else {
            Pass "ziplark-gui.exe still running after 10s (pid $($proc.Id))"
            $proc.Kill()
            $proc.WaitForExit(5000) | Out-Null
        }
    } catch {
        Fail "GUI launch: $_"
    }
}

Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue

Write-Host ""
if ($script:Failures.Count) {
    Write-Host "FAILED ($($script:Failures.Count))" -ForegroundColor Red
    $script:Failures | ForEach-Object { Write-Host " - $_" -ForegroundColor Red }
    exit 1
}
Write-Host "All Windows smoke checks passed." -ForegroundColor Green
exit 0
