; "Open in Koden" shell verbs for folders, folder backgrounds, and drives.
; HKCU matches installer currentUser scope. %V = clicked path.
; NoWorkingDirectory keeps Explorer from overriding %V (System32 on Drive).

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Classes\Directory\shell\OpenInKoden" "" "Open in Koden"
  WriteRegStr HKCU "Software\Classes\Directory\shell\OpenInKoden" "Icon" '"$INSTDIR\koden.exe",0'
  WriteRegStr HKCU "Software\Classes\Directory\shell\OpenInKoden" "NoWorkingDirectory" ""
  WriteRegStr HKCU "Software\Classes\Directory\shell\OpenInKoden\command" "" '"$INSTDIR\koden.exe" "%V"'

  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\OpenInKoden" "" "Open in Koden"
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\OpenInKoden" "Icon" '"$INSTDIR\koden.exe",0'
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\OpenInKoden" "NoWorkingDirectory" ""
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\OpenInKoden\command" "" '"$INSTDIR\koden.exe" "%V"'

  WriteRegStr HKCU "Software\Classes\Drive\shell\OpenInKoden" "" "Open in Koden"
  WriteRegStr HKCU "Software\Classes\Drive\shell\OpenInKoden" "Icon" '"$INSTDIR\koden.exe",0'
  WriteRegStr HKCU "Software\Classes\Drive\shell\OpenInKoden" "NoWorkingDirectory" ""
  WriteRegStr HKCU "Software\Classes\Drive\shell\OpenInKoden\command" "" '"$INSTDIR\koden.exe" "%V"'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey HKCU "Software\Classes\Directory\shell\OpenInKoden"
  DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\OpenInKoden"
  DeleteRegKey HKCU "Software\Classes\Drive\shell\OpenInKoden"
!macroend
