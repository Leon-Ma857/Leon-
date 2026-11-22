; Tauri's NSIS template only copies the exe. The loader has to sit beside it,
; or Windows reports "WebView2Loader.dll was not found" on launch.
!macro NSIS_HOOK_POSTINSTALL
  !searchparse "${MAINBINARYSRCPATH}" "" LUMA_BINARY_DIR "snaptrans.exe"
  SetOutPath "$INSTDIR"
  File "${LUMA_BINARY_DIR}WebView2Loader.dll"
  !undef LUMA_BINARY_DIR
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\WebView2Loader.dll"
!macroend
