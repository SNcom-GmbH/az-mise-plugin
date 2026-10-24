$ErrorActionPreference = "Stop"

& $env:AZURE_AC_MISE -C $env:AZURE_AC_PLUGIN exec uv@0.13.4 -- uv run --quiet --no-dev --inexact --locked --project $env:AZURE_AC_PLUGIN python $env:AZURE_AC_HELPER hook
exit $LASTEXITCODE
