Option Explicit

Dim WshShell, fso, strDir, batPath, response, opt, runCmd
Set WshShell = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")

strDir = Left(WScript.ScriptFullName, InStrRev(WScript.ScriptFullName, "\"))
WshShell.CurrentDirectory = strDir
batPath = strDir & "restart_bot.bat"

If Not fso.FileExists(batPath) Then
    MsgBox "Cannot find restart_bot.bat" & vbCrLf & "Path: " & batPath, vbCritical, "TG Bot Manager"
    WScript.Quit 1
End If

response = MsgBox("Select restart mode:" & vbCrLf & vbCrLf & "[Yes] Quick Restart" & vbCrLf & "[No] Safe Restart", vbYesNoCancel + vbQuestion, "TG Bot Restart")
If response = vbCancel Then WScript.Quit 0

If response = vbYes Then
    opt = "1"
Else
    opt = "2"
End If

runCmd = "cmd.exe /k """ & batPath & """ " & opt
WshShell.Run runCmd, 1, False

MsgBox "Restart command sent." & vbCrLf & "Admin Panel: http://127.0.0.1:8080", vbInformation, "TG Bot Manager"
