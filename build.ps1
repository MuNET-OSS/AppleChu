[CmdletBinding()]
param(
    [Parameter()]
    [string]$Deploy
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = $PSScriptRoot
$target = Join-Path $root 'target'
$exampleName = 'AppleChu.example.toml'
$artifacts = @(
    @{
        Name = 'winhttp.dll'
        Package = 'applechu'
        Triple = 'i686-pc-windows-msvc'
    },
    @{
        Name = 'winmm.dll'
        Package = 'applechu-amdaemon'
        Triple = 'x86_64-pc-windows-msvc'
    }
)

Push-Location $root
try {
    foreach ($artifact in $artifacts) {
        Write-Host "正在构建 $($artifact.Name)..."
        & cargo build --release --package $artifact.Package --target $artifact.Triple
        if ($LASTEXITCODE -ne 0) {
            throw "$($artifact.Name) 构建失败，退出代码：$LASTEXITCODE"
        }

        $source = Join-Path $target "$($artifact.Triple)\release\$($artifact.Name)"
        $destination = Join-Path $target $artifact.Name
        Copy-Item -LiteralPath $source -Destination $destination -Force
        Write-Host "已复制到 $destination"
    }

    $exampleSource = Join-Path $target "i686-pc-windows-msvc\release\$exampleName"
    & cargo run --package applechu-schema --bin verify_pe --target x86_64-pc-windows-msvc -- `
        (Join-Path $target 'winhttp.dll') $exampleSource
    if ($LASTEXITCODE -ne 0) {
        throw "示例配置生成失败，退出代码：$LASTEXITCODE"
    }
    $exampleDestination = Join-Path $target $exampleName
    Copy-Item -LiteralPath $exampleSource -Destination $exampleDestination -Force
    Write-Host "已复制到 $exampleDestination"

    if ($Deploy) {
        $deployDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Deploy)
        New-Item -ItemType Directory -Path $deployDirectory -Force | Out-Null

        foreach ($artifact in $artifacts) {
            $source = Join-Path $target $artifact.Name
            Copy-Item -LiteralPath $source -Destination $deployDirectory -Force
        }
        Copy-Item -LiteralPath $exampleDestination -Destination $deployDirectory -Force

        Write-Host "已部署到 $deployDirectory"
    }
}
finally {
    Pop-Location
}
