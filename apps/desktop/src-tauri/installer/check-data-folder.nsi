Unicode true
!include MUI2.nsh

!define PRODUCTNAME "Commy"
!define BUNDLEID "chat.commy.desktop"

Name "${PRODUCTNAME}"
OutFile "${OUTPUT}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\${PRODUCTNAME}"

Var PassiveMode
Var UpdateMode
Var DeleteAppDataCheckboxState

!insertmacro MUI_PAGE_DIRECTORY
!include "data-folder.nsh"
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  StrCpy $PassiveMode 0
  StrCpy $UpdateMode 0
FunctionEnd

Function un.onInit
  StrCpy $UpdateMode 0
  StrCpy $DeleteAppDataCheckboxState 1
FunctionEnd

Section Install
  SetOutPath $INSTDIR
  !insertmacro NSIS_HOOK_POSTINSTALL
  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

Section Uninstall
  !insertmacro NSIS_HOOK_PREUNINSTALL
  !insertmacro NSIS_HOOK_POSTUNINSTALL
SectionEnd
