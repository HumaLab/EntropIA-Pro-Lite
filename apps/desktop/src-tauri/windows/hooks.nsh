!macro NSIS_HOOK_PREINSTALL
  ; NSIS overwrites files but never removes the ones a newer version stopped
  ; shipping. Lite dropped the local-ml payload in 1.0.17 (tauri.lite.windows.conf.json),
  ; so clear it before copying; Pro copies these right back in its own install.
  ; The runtime Pro downloads lives in the app data dir, not here.
  StrCmp "$INSTDIR" "" preinstall_done
  RMDir /r "$INSTDIR\resources\models"
  RMDir /r "$INSTDIR\resources\tools"
  RMDir /r "$INSTDIR\resources\runtime-pack"
  RMDir /r "$INSTDIR\scripts"
  Delete "$INSTDIR\resources\README.md"
  Delete "$INSTDIR\resources\README.en.md"
  preinstall_done:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  SetOutPath "$INSTDIR"
  CopyFiles /SILENT "$INSTDIR\target\release\vc-runtime\msvcp140.dll" "$INSTDIR\msvcp140.dll"
  CopyFiles /SILENT "$INSTDIR\target\release\vc-runtime\msvcp140_1.dll" "$INSTDIR\msvcp140_1.dll"
  CopyFiles /SILENT "$INSTDIR\target\release\vc-runtime\vcomp140.dll" "$INSTDIR\vcomp140.dll"
  CopyFiles /SILENT "$INSTDIR\target\release\vc-runtime\vcruntime140.dll" "$INSTDIR\vcruntime140.dll"
  CopyFiles /SILENT "$INSTDIR\target\release\vc-runtime\vcruntime140_1.dll" "$INSTDIR\vcruntime140_1.dll"
  IfFileExists "$INSTDIR\target\release\vc-runtime\concrt140.dll" 0 +2
    CopyFiles /SILENT "$INSTDIR\target\release\vc-runtime\concrt140.dll" "$INSTDIR\concrt140.dll"

  ; Windows Sandbox can expose the WebView2 registry key while the runtime is
  ; still unavailable to the app user. Tauri's default NSIS section skips the
  ; bootstrapper in that state, so force the evergreen bootstrapper once more.
  Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
  File "/oname=$TEMP\MicrosoftEdgeWebview2Setup.exe" "${WEBVIEW2BOOTSTRAPPERPATH}"
  ExecWait '"$TEMP\MicrosoftEdgeWebview2Setup.exe" /silent /install'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Delete "$INSTDIR\concrt140.dll"
  Delete "$INSTDIR\msvcp140.dll"
  Delete "$INSTDIR\msvcp140_1.dll"
  Delete "$INSTDIR\vcomp140.dll"
  Delete "$INSTDIR\vcruntime140.dll"
  Delete "$INSTDIR\vcruntime140_1.dll"
!macroend
