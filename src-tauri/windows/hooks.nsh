!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec `powershell -NoProfile -ExecutionPolicy Bypass -Command "Add-AppxPackage -Path '$INSTDIR\idleport-identity.msix' -ExternalLocation '$INSTDIR' -ForceUpdateFromAnyVersion"`
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec `powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-AppxPackage -Name IdlePort | Remove-AppxPackage"`
!macroend
