!include nsDialogs.nsh
!include LogicLib.nsh

!define DATA_FOLDER_POINTER "data-folder.txt"
!define DATA_FOLDER_NAME "Commy"
!define DATA_FOLDER_BOM 65279

Var DataFolder
Var DataFolderField
Var DataFolderStandard
Var DataFolderToRemove

Function DataFolderTrim
  Exch $R0
  Push $R1
  StrCpy $R1 $R0 1 -1
  ${If} $R1 == "\"
  ${AndIf} $R0 != ""
    StrCpy $R0 $R0 -1
  ${EndIf}
  Pop $R1
  Exch $R0
FunctionEnd

Function DataFolderNamed
  Exch $R0
  Push $R1
  StrCpy $R1 $R0 6 -6
  ${If} $R1 != "\${DATA_FOLDER_NAME}"
    StrCpy $R0 "$R0\${DATA_FOLDER_NAME}"
  ${EndIf}
  Pop $R1
  Exch $R0
FunctionEnd

Function DataFolderIsEmpty
  Exch $R0
  Push $R1
  Push $R2
  Push $R3
  StrCpy $R1 1
  FindFirst $R2 $R3 "$R0\*.*"
  ${DoWhile} $R3 != ""
    ${If} $R3 != "."
    ${AndIf} $R3 != ".."
      StrCpy $R1 0
      ${Break}
    ${EndIf}
    FindNext $R2 $R3
  ${Loop}
  FindClose $R2
  StrCpy $R0 $R1
  Pop $R3
  Pop $R2
  Pop $R1
  Exch $R0
FunctionEnd

Function DataFolderPage
  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    Abort
  ${EndIf}
  StrCpy $DataFolderStandard "$APPDATA\${BUNDLEID}"
  ${If} ${FileExists} "$DataFolderStandard\*.*"
    Abort
  ${EndIf}
  ${If} $DataFolder == ""
    StrCpy $DataFolder $DataFolderStandard
  ${EndIf}
  !insertmacro MUI_HEADER_TEXT "Data Folder" "Choose where ${PRODUCTNAME} keeps your messages, settings and the files you share."
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}
  ${NSD_CreateLabel} 0 0 100% 30u "${PRODUCTNAME} keeps its data in the folder below. Pick another drive if this one is short on space. You can move it later in Settings, under Storage."
  Pop $0
  ${NSD_CreateDirRequest} 0 40u 74% 13u "$DataFolder"
  Pop $DataFolderField
  ${NSD_CreateBrowseButton} 76% 39u 24% 15u "Browse..."
  Pop $0
  ${NSD_OnClick} $0 DataFolderBrowse
  ${NSD_CreateButton} 0 62u 44% 15u "Use the Standard Folder"
  Pop $0
  ${NSD_OnClick} $0 DataFolderUseStandard
  nsDialogs::Show
FunctionEnd

Function DataFolderBrowse
  Pop $0
  ${NSD_GetText} $DataFolderField $1
  nsDialogs::SelectFolderDialog "Choose a folder for ${PRODUCTNAME}'s data" "$1"
  Pop $1
  ${If} $1 == error
    Return
  ${EndIf}
  Push $1
  Call DataFolderTrim
  Pop $1
  ${If} $1 != $DataFolderStandard
    Push $1
    Call DataFolderNamed
    Pop $1
  ${EndIf}
  ${NSD_SetText} $DataFolderField $1
FunctionEnd

Function DataFolderUseStandard
  Pop $0
  ${NSD_SetText} $DataFolderField $DataFolderStandard
FunctionEnd

Function DataFolderPageLeave
  ${NSD_GetText} $DataFolderField $0
  Push $0
  Call DataFolderTrim
  Pop $0
  ${If} $0 == ""
  ${OrIf} $0 == $DataFolderStandard
    StrCpy $DataFolder $DataFolderStandard
    Return
  ${EndIf}
  StrCpy $1 $0 1 1
  StrCpy $2 $0 2
  ${If} $1 != ":"
  ${AndIf} $2 != "\\"
    MessageBox MB_ICONEXCLAMATION|MB_OK "Enter a full folder path, such as D:\${DATA_FOLDER_NAME}."
    Abort
  ${EndIf}
  StrLen $1 $0
  ${If} $1 <= 3
    Push $0
    Call DataFolderNamed
    Pop $0
  ${EndIf}
  ${If} ${FileExists} "$0\*.*"
    Push $0
    Call DataFolderIsEmpty
    Pop $1
    ${If} $1 != 1
      Push $0
      Call DataFolderNamed
      Pop $0
    ${EndIf}
  ${EndIf}
  ${If} ${FileExists} "$0\*.*"
    Push $0
    Call DataFolderIsEmpty
    Pop $1
    ${If} $1 != 1
      MessageBox MB_ICONEXCLAMATION|MB_OK "$0 already has files in it. Choose an empty folder."
      Abort
    ${EndIf}
  ${EndIf}
  ClearErrors
  CreateDirectory "$0"
  FileOpen $1 "$0\.commy-write-check" w
  ${If} ${Errors}
    MessageBox MB_ICONEXCLAMATION|MB_OK "${PRODUCTNAME} cannot save files in $0. Choose another folder."
    Abort
  ${EndIf}
  FileClose $1
  Delete "$0\.commy-write-check"
  RMDir "$0"
  StrCpy $DataFolder $0
  ${NSD_SetText} $DataFolderField $0
FunctionEnd

Page custom DataFolderPage DataFolderPageLeave

!macro NSIS_HOOK_POSTINSTALL
  ${If} $DataFolder != ""
  ${AndIf} $DataFolder != "$APPDATA\${BUNDLEID}"
    CreateDirectory "$DataFolder"
    CreateDirectory "$APPDATA\${BUNDLEID}"
    FileOpen $0 "$APPDATA\${BUNDLEID}\${DATA_FOLDER_POINTER}" w
    FileWriteWord $0 ${DATA_FOLDER_BOM}
    FileWriteUTF16LE $0 "$DataFolder"
    FileClose $0
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  StrCpy $DataFolderToRemove ""
  ClearErrors
  FileOpen $0 "$APPDATA\${BUNDLEID}\${DATA_FOLDER_POINTER}" r
  ${IfNot} ${Errors}
    FileReadWord $0 $1
    ${If} $1 = ${DATA_FOLDER_BOM}
      FileReadUTF16LE $0 $DataFolderToRemove
    ${EndIf}
    FileClose $0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  StrLen $1 $DataFolderToRemove
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
  ${AndIf} $1 > 3
  ${AndIf} ${FileExists} "$DataFolderToRemove\running.lock"
    RMDir /r "$DataFolderToRemove"
  ${EndIf}
!macroend
