; Installer of the native Nimbus: the app, its streaming server (LiveKit + token service).
; Per user, no administrator rights.
; Build:  powershell -File installer\build.ps1
Unicode true
!include "MUI2.nsh"

!ifndef EXE
  !error "pass /DEXE=... /DSERVER=... /DICON=... /DOUT=..."
!endif

Name "Nimbus"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Nimbus Native"
InstallDirRegKey HKCU "Software\Nimbus Native" "dir"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_FINISHPAGE_RUN "$INSTDIR\nimbus-native.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Ð—Ð°Ð¿ÑƒÑÑ‚Ð¸Ñ‚ÑŒ Nimbus"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Russian"

; A quiet install (the one the updater runs) starts the new version itself when it is done.
Function .onInstSuccess
  IfSilent 0 +2
  Exec "$INSTDIR\nimbus-native.exe"
FunctionEnd

Section "Nimbus"
  ; A running copy would keep its files locked.
  nsExec::Exec 'taskkill /IM nimbus-native.exe /F'
  nsExec::Exec 'taskkill /IM livekit-server.exe /F'
  nsExec::Exec 'taskkill /IM nimbus-token.exe /F'
  Sleep 800

  SetOutPath "$INSTDIR"
  File "${EXE}"

  ; The streaming server. Keys are made on the first start, so none are shipped.
  SetOutPath "$INSTDIR\server"
  File "${SERVER}\nimbus-token.exe"
  SetOutPath "$INSTDIR\server\livekit"
  File "${SERVER}\livekit\livekit-server.exe"

  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Nimbus Native" "dir" "$INSTDIR"

  CreateDirectory "$SMPROGRAMS\Nimbus"
  CreateShortcut "$SMPROGRAMS\Nimbus\Nimbus.lnk" "$INSTDIR\nimbus-native.exe"
  CreateShortcut "$DESKTOP\Nimbus.lnk" "$INSTDIR\nimbus-native.exe"
  ; Windows keeps icons by path: tell it to read them again, so an update shows a new icon at once.
  System::Call "shell32::SHChangeNotify(i 0x08000000, i 0, i 0, i 0)"

  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NimbusNative" "DisplayName" "Nimbus"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NimbusNative" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NimbusNative" "DisplayIcon" "$INSTDIR\nimbus-native.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NimbusNative" "Publisher" "Nimbus"
SectionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /IM nimbus-native.exe /F'
  nsExec::Exec 'taskkill /IM livekit-server.exe /F'
  nsExec::Exec 'taskkill /IM nimbus-token.exe /F'
  Sleep 800
  Delete "$SMPROGRAMS\Nimbus\Nimbus.lnk"
  RMDir "$SMPROGRAMS\Nimbus"
  Delete "$DESKTOP\Nimbus.lnk"
  RMDir /r "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\NimbusNative"
  DeleteRegKey HKCU "Software\Nimbus Native"
SectionEnd
