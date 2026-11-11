$ErrorActionPreference = 'Stop'

$root = $PSScriptRoot
$staging = Join-Path $root '..\target\identity'
$output = Join-Path $root 'idleport-identity.msix'
$sdk = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\makeappx.exe' |
    Where-Object { Test-Path (Join-Path $_.DirectoryName 'signtool.exe') } |
    Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1
if (-not $sdk) { throw 'Windows SDK não encontrado (makeappx/signtool).' }
$makeappx = $sdk.FullName
$signtool = Join-Path $sdk.DirectoryName 'signtool.exe'

$version = (Get-Content (Join-Path $root '..\tauri.conf.json') -Raw | ConvertFrom-Json).version
$manifest = [xml](Get-Content (Join-Path $root 'identity\AppxManifest.xml') -Raw)
$manifest.Package.Identity.Version = "$version.0"
$publisher = $manifest.Package.Identity.Publisher

if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
New-Item -ItemType Directory -Force $staging | Out-Null
Copy-Item (Join-Path $root 'identity\assets') $staging -Recurse
$manifest.Save((Join-Path $staging 'AppxManifest.xml'))

$pfx = $env:IDLEPORT_CERT_PFX
$password = $env:IDLEPORT_CERT_PASSWORD
if (-not $pfx) {
    $pfx = Join-Path $root 'dev-cert.pfx'
    $password = 'idleport-dev'
    if (-not (Test-Path $pfx)) {
        $cert = New-SelfSignedCertificate -Type Custom -Subject $publisher -KeyUsage DigitalSignature `
            -FriendlyName 'IdlePort Dev' -CertStoreLocation 'Cert:\CurrentUser\My' `
            -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
        Export-PfxCertificate -Cert $cert -FilePath $pfx -Password (ConvertTo-SecureString $password -AsPlainText -Force) | Out-Null
        Export-Certificate -Cert $cert -FilePath (Join-Path $root 'dev-cert.cer') | Out-Null
        Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)"
    }
}

$subject = (Get-PfxData -FilePath $pfx -Password (ConvertTo-SecureString $password -AsPlainText -Force)).EndEntityCertificates[0].Subject
if ($subject -ne $publisher) {
    throw "O certificado ($subject) não bate com o Publisher do AppxManifest e do app.manifest ($publisher)."
}

& $makeappx pack /d $staging /p $output /nv /o | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'makeappx falhou.' }
& $signtool sign /fd SHA256 /f $pfx /p $password $output | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'signtool falhou.' }
Write-Host "Identidade empacotada: $output ($publisher)"
