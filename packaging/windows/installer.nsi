; NSIS installer for Atomic Chat. Build:  makensis /DVERSION=1.0.0 packaging/windows/installer.nsi  (run from the repo root)
!include "MUI2.nsh"
Name "Atomic Chat"
OutFile "..\..\dist\Atomic_Chat-${VERSION}-x64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Atomic Chat"
RequestExecutionLevel user
!define MUI_ICON "..\..\assets\icon\atomic-chat.ico"
!define MUI_UNICON "..\..\assets\icon\atomic-chat.ico"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetOutPath "$INSTDIR"
  File "..\..\dist\Atomic_Chat-${VERSION}-x64-portable.exe"
  Rename "$INSTDIR\Atomic_Chat-${VERSION}-x64-portable.exe" "$INSTDIR\atomic-chat.exe"
  File "..\..\assets\icon\atomic-chat.ico"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Atomic Chat.lnk" "$INSTDIR\atomic-chat.exe" "" "$INSTDIR\atomic-chat.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AtomicChat" "DisplayName" "Atomic Chat"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AtomicChat" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AtomicChat" "DisplayIcon" "$INSTDIR\atomic-chat.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AtomicChat" "UninstallString" "$INSTDIR\uninstall.exe"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\atomic-chat.exe"
  Delete "$INSTDIR\atomic-chat.ico"
  Delete "$INSTDIR\uninstall.exe"
  Delete "$SMPROGRAMS\Atomic Chat.lnk"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AtomicChat"
SectionEnd
