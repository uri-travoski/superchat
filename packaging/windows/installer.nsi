; NSIS installer for SuperChat. Build:  makensis /DVERSION=1.1.1 packaging/windows/installer.nsi  (run from the repo root)
!include "MUI2.nsh"
Name "SuperChat"
OutFile "..\..\dist\superchat-${VERSION}-x64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\SuperChat"
RequestExecutionLevel user
!define MUI_ICON "..\..\assets\icon\superchat.ico"
!define MUI_UNICON "..\..\assets\icon\superchat.ico"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetOutPath "$INSTDIR"
  File "..\..\dist\superchat-${VERSION}-x64-portable.exe"
  Rename "$INSTDIR\superchat-${VERSION}-x64-portable.exe" "$INSTDIR\superchat.exe"
  File "..\..\assets\icon\superchat.ico"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\SuperChat.lnk" "$INSTDIR\superchat.exe" "" "$INSTDIR\superchat.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\SuperChat" "DisplayName" "SuperChat"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\SuperChat" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\SuperChat" "DisplayIcon" "$INSTDIR\superchat.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\SuperChat" "UninstallString" "$INSTDIR\uninstall.exe"
  ; superchat:// links (the login page's "Open in SuperChat" button) start the app
  WriteRegStr HKCU "Software\Classes\superchat" "" "URL:SuperChat"
  WriteRegStr HKCU "Software\Classes\superchat" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\superchat\DefaultIcon" "" "$INSTDIR\superchat.ico"
  WriteRegStr HKCU "Software\Classes\superchat\shell\open\command" "" '"$INSTDIR\superchat.exe" "%1"'
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\superchat.exe"
  Delete "$INSTDIR\superchat.ico"
  Delete "$INSTDIR\uninstall.exe"
  Delete "$SMPROGRAMS\SuperChat.lnk"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\SuperChat"
  DeleteRegKey HKCU "Software\Classes\superchat"
SectionEnd
