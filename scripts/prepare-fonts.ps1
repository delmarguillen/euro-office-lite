# Euro-Office Lite - Font preparation script
# Downloads Liberation Sans/Serif/Mono + Carlito, copies TTFs, generates AllFonts.js
#
# Usage: powershell -ExecutionPolicy Bypass -File scripts\prepare-fonts.ps1

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$fontsDir = Join-Path $projectRoot "src\fonts"
$allFontsPath = Join-Path $projectRoot "src\sdkjs\common\AllFonts.js"
$tempDir = Join-Path $env:TEMP "euro-office-fonts"

# Fonts are stored as plain TTF (no XOR encoding needed).
# Local/common.js overrides LoadFontAsync and does NOT apply XOR decode.

$liberationVersion = "2.1.5"
$liberationUrl = "https://github.com/liberationfonts/liberation-fonts/files/7261482/liberation-fonts-ttf-2.1.5.tar.gz"
# Carlito 1.104; a commit, not a branch, so the fonts cannot change under us.
$carlitoUrl = "https://github.com/googlefonts/carlito/raw/3a810cab78ebd6e2e4eed42af9e8453c4f9b850a/fonts/ttf/Carlito-Regular.ttf"
$carlitoUrlBase = "https://github.com/googlefonts/carlito/raw/3a810cab78ebd6e2e4eed42af9e8453c4f9b850a/fonts/ttf"

# sha256 of every downloaded file; must match the table in prepare-fonts.sh.
$liberationTarSha256 = "7191c669bf38899f73a2094ed00f7b800553364f90e2637010a69c0e268f25d0"
$fontSha256 = [ordered]@{
    "LiberationSans-Regular.ttf"     = "76d04c18ea243f426b7de1f3ad208e927008f961dc5945e5aad352d0dfde8ee8"
    "LiberationSans-Italic.ttf"      = "e5bae5c4cde31f22142753855f4f8fb86da6ff39955ed3c0a11248b0d16948b0"
    "LiberationSans-Bold.ttf"        = "788abee4c806d660e8aee46689dd8540cd4bb98da03dcc9d171ce3efd99a9173"
    "LiberationSans-BoldItalic.ttf"  = "698da70fc191cc5f33ad4d6d3fe830fe4624b898ea2e3169955928b7c491f1ee"
    "LiberationSerif-Regular.ttf"    = "058ea80864aef09a23f45cbec2bb5400bc3dfbdea01c3f10538a21fcb497fb74"
    "LiberationSerif-Italic.ttf"     = "0e3dea9f8d613e006ccfa62201f33e265d19167bd0907725c3e145368b04fc2e"
    "LiberationSerif-Bold.ttf"       = "d754ba427cfe0bca54ae052384baa8f842da5bd6550ad4da024ac441e7a7d5ce"
    "LiberationSerif-BoldItalic.ttf" = "f17db8af71e24d2066b587546021d4f0b296be389512b658dec3c09affeb11a7"
    "LiberationMono-Regular.ttf"     = "f2b83c763e8afd21709333370bed4774337fae82267937e2b5aea7e2fbd922c1"
    "LiberationMono-Italic.ttf"      = "605c01c711b44480a7508d349dfbf3264e81fa43d69e61cfa7d10b86e764c4d1"
    "LiberationMono-Bold.ttf"        = "bd62a0672d0b9b6710b01df434c80ad54fa5f0835207eb7b17b7a761463067bb"
    "LiberationMono-BoldItalic.ttf"  = "79451f3c09fe25116098853b7a2ca6e2436220ccc11af022979adbcf195be130"
    "Carlito-Regular.ttf"            = "f6418f708baede9789daef5d458c0f53d2a888af9820e8062934e504fedc6595"
    "Carlito-Italic.ttf"             = "0b019225e58d702bfedcbd35c21696769f8ee115cb6343f84c2f240312450d1c"
    "Carlito-Bold.ttf"               = "bb5d20f79b82599ec72983597437373a80f2d2085fa91fc144fd74e876a594db"
    "Carlito-BoldItalic.ttf"         = "b32928186c119599e03ca6a1ffc680fdcb7fac95772f4b95d989cf6cd3861517"
}

function Test-Sha256($path, $expected) {
    return (Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant() -eq $expected
}

# Font definitions: Liberation Sans, Serif, Mono + Carlito
$liberationFonts = @(
    @{ File = "LiberationSans-Regular.ttf";    Family = "Liberation Sans";  Style = "R" },
    @{ File = "LiberationSans-Italic.ttf";     Family = "Liberation Sans";  Style = "I" },
    @{ File = "LiberationSans-Bold.ttf";       Family = "Liberation Sans";  Style = "B" },
    @{ File = "LiberationSans-BoldItalic.ttf"; Family = "Liberation Sans";  Style = "BI" },
    @{ File = "LiberationSerif-Regular.ttf";    Family = "Liberation Serif"; Style = "R" },
    @{ File = "LiberationSerif-Italic.ttf";     Family = "Liberation Serif"; Style = "I" },
    @{ File = "LiberationSerif-Bold.ttf";       Family = "Liberation Serif"; Style = "B" },
    @{ File = "LiberationSerif-BoldItalic.ttf"; Family = "Liberation Serif"; Style = "BI" },
    @{ File = "LiberationMono-Regular.ttf";     Family = "Liberation Mono";  Style = "R" },
    @{ File = "LiberationMono-Italic.ttf";      Family = "Liberation Mono";  Style = "I" },
    @{ File = "LiberationMono-Bold.ttf";        Family = "Liberation Mono";  Style = "B" },
    @{ File = "LiberationMono-BoldItalic.ttf";  Family = "Liberation Mono";  Style = "BI" }
)

$carlitoFonts = @(
    @{ File = "Carlito-Regular.ttf";    Family = "Carlito"; Style = "R" },
    @{ File = "Carlito-Italic.ttf";     Family = "Carlito"; Style = "I" },
    @{ File = "Carlito-Bold.ttf";       Family = "Carlito"; Style = "B" },
    @{ File = "Carlito-BoldItalic.ttf"; Family = "Carlito"; Style = "BI" }
)

$allFontDefs = $liberationFonts + $carlitoFonts

Write-Host ""
Write-Host "=== Euro-Office Lite - Font Preparation ===" -ForegroundColor Cyan

# 1. Download Liberation fonts (Sans + Serif + Mono are all in the same archive)
if (-not (Test-Path $tempDir)) { New-Item -ItemType Directory -Path $tempDir | Out-Null }
$tarGzPath = Join-Path $tempDir "liberation-fonts.tar.gz"

if (-not (Test-Path $tarGzPath)) {
    Write-Host "[1/6] Downloading Liberation fonts..." -ForegroundColor Yellow
    Invoke-WebRequest -Uri $liberationUrl -OutFile $tarGzPath -UseBasicParsing
} else {
    Write-Host "[1/6] Using cached Liberation download..." -ForegroundColor DarkGray
}
if (-not (Test-Sha256 $tarGzPath $liberationTarSha256)) {
    Remove-Item $tarGzPath -Force
    throw "sha256 mismatch for the Liberation archive"
}

# 2. Extract Liberation archive
Write-Host "[2/6] Extracting Liberation fonts..." -ForegroundColor Yellow
$extractDir = Join-Path $tempDir "extracted"
if (Test-Path $extractDir) { Remove-Item -Recurse -Force $extractDir }
New-Item -ItemType Directory -Path $extractDir | Out-Null
tar -xzf $tarGzPath -C $extractDir

$ttfDir = Get-ChildItem -Path $extractDir -Recurse -Directory | Where-Object { $_.Name -match "liberation-fonts-ttf" } | Select-Object -First 1
if (-not $ttfDir) {
    throw "Could not find extracted font directory"
}

# 3. Download Carlito fonts
Write-Host "[3/6] Downloading Carlito fonts..." -ForegroundColor Yellow
$carlitoDir = Join-Path $tempDir "carlito"
if (-not (Test-Path $carlitoDir)) { New-Item -ItemType Directory -Path $carlitoDir | Out-Null }

foreach ($cf in $carlitoFonts) {
    $destPath = Join-Path $carlitoDir $cf.File
    if (-not (Test-Path $destPath)) {
        $url = "$carlitoUrlBase/$($cf.File)"
        Write-Host ("  Downloading: " + $cf.File) -ForegroundColor DarkYellow
        Invoke-WebRequest -Uri $url -OutFile $destPath -UseBasicParsing
    } else {
        Write-Host ("  Cached: " + $cf.File) -ForegroundColor DarkGray
    }
}

# 4. Copy all TTF files to src/fonts/
Write-Host "[4/6] Copying fonts..." -ForegroundColor Yellow
if (-not (Test-Path $fontsDir)) { New-Item -ItemType Directory -Path $fontsDir | Out-Null }

foreach ($def in $liberationFonts) {
    $srcFile = Join-Path $ttfDir.FullName $def.File
    if (-not (Test-Path $srcFile)) {
        throw ("Font file not found: " + $srcFile)
    }
    $destFile = Join-Path $fontsDir $def.File
    Copy-Item -Path $srcFile -Destination $destFile -Force
    Write-Host ("  Copied: " + $def.File) -ForegroundColor DarkGreen
}

foreach ($def in $carlitoFonts) {
    $srcFile = Join-Path $carlitoDir $def.File
    if (-not (Test-Path $srcFile)) {
        throw ("Carlito font not found: " + $srcFile)
    }
    $destFile = Join-Path $fontsDir $def.File
    Copy-Item -Path $srcFile -Destination $destFile -Force
    Write-Host ("  Copied: " + $def.File) -ForegroundColor DarkGreen
}

foreach ($name in $fontSha256.Keys) {
    if (-not (Test-Sha256 (Join-Path $fontsDir $name) $fontSha256[$name])) {
        throw ("sha256 mismatch for " + $name + " (a stale download in " + $tempDir + " can cause this)")
    }
}

# 5. Generate AllFonts.js
Write-Host "[5/6] Generating AllFonts.js..." -ForegroundColor Yellow

# Build __fonts_files array (all font files in order)
$filesList = @()
foreach ($def in $allFontDefs) {
    $filesList += ('"' + $def.File + '"')
}
$filesArray = $filesList -join ", "

# Build __fonts_infos array
# Format: [name, indexR, faceR, indexI, faceI, indexB, faceB, indexBI, faceBI]
# Group fonts by family and find indices
$families = @{}
for ($i = 0; $i -lt $allFontDefs.Count; $i++) {
    $fam = $allFontDefs[$i].Family
    if (-not $families.ContainsKey($fam)) {
        $families[$fam] = @{ R = -1; I = -1; B = -1; BI = -1 }
    }
    switch ($allFontDefs[$i].Style) {
        "R"  { $families[$fam].R  = $i }
        "I"  { $families[$fam].I  = $i }
        "B"  { $families[$fam].B  = $i }
        "BI" { $families[$fam].BI = $i }
    }
}

function MakeInfoEntry($name, $idx) {
    return '["' + $name + '", ' + $idx.R + ', 0, ' + $idx.I + ', 0, ' + $idx.B + ', 0, ' + $idx.BI + ', 0]'
}

$sansIdx  = $families["Liberation Sans"]
$serifIdx = $families["Liberation Serif"]
$monoIdx  = $families["Liberation Mono"]
$carlitoIdx = $families["Carlito"]

$infos = @(
    (MakeInfoEntry "Liberation Sans"  $sansIdx),
    (MakeInfoEntry "Liberation Serif" $serifIdx),
    (MakeInfoEntry "Liberation Mono"  $monoIdx),
    (MakeInfoEntry "Carlito"          $carlitoIdx),
    (MakeInfoEntry "Arial"            $sansIdx),
    (MakeInfoEntry "Helvetica"        $sansIdx),
    (MakeInfoEntry "Times New Roman"  $serifIdx),
    (MakeInfoEntry "Courier New"      $monoIdx),
    (MakeInfoEntry "Calibri"          $carlitoIdx),
    (MakeInfoEntry "Cambria"          $serifIdx)
)

$nl = [char]10
$content = '// Generated by scripts/prepare-fonts.ps1 -- do not edit manually.' + $nl
$content += 'window["__fonts_files"] = [' + $filesArray + '];' + $nl
$content += 'window["__fonts_infos"] = [' + $nl
for ($i = 0; $i -lt $infos.Count; $i++) {
    $comma = if ($i -lt $infos.Count - 1) { "," } else { "" }
    $content += '    ' + $infos[$i] + $comma + $nl
}
$content += '];' + $nl
$content += 'window["g_fonts_selection_bin"] = "";' + $nl

[System.IO.File]::WriteAllText($allFontsPath, $content, [System.Text.UTF8Encoding]::new($false))

# 6. Generate font thumbnail sprites
Write-Host "[6/6] Generating font thumbnail sprites..." -ForegroundColor Yellow
$spriteScript = Join-Path $projectRoot "scripts\generate-font-sprite.py"
python $spriteScript

Write-Host ""
Write-Host "=== Font preparation complete ===" -ForegroundColor Green
Write-Host ("Fonts in: " + $fontsDir)
Write-Host ("AllFonts.js updated: " + $allFontsPath)
Write-Host ""
