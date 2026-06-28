
function ParseZedWorkspace {
    $metadata = cargo metadata --no-deps --offline | ConvertFrom-Json
    $env:TAU_WORKSPACE = $metadata.workspace_root
    $env:RELEASE_VERSION = $metadata.packages | Where-Object { $_.name -eq "tau" } | Select-Object -ExpandProperty version
}
