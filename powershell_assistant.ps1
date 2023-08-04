Set-PSReadlineKeyHandler -Key 'Ctrl+x' -ScriptBlock {
    # Generate a unique temporary file name
    $tempFileName = [System.IO.Path]::GetTempFileName()
    # $tempFileName = "./testfile"
    $bufferState = $cursorState = $null
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref] $bufferState, [ref] $cursorState)
    # Write the buffer content to the temporary file
    $bufferState | Out-File -FilePath $tempFileName -Force
    # Remove the file extension from the temporary file name
    # $tempFileBaseName = [System.IO.Path]::GetFileNameWithoutExtension($tempFileName)
    # Run the CLI application with the temporary file
    Start-Process shai -ArgumentList "ask --model open-aigpt35turbo --edit-file $tempFileName" -Wait
    $fileContents = Get-Content -Raw -Path $tempFileName
    # # Remove the temporary file
    Remove-Item -Path $tempFileName -Force
    [Microsoft.PowerShell.PSConsoleReadLine]::BackwardKillLine()
    [Microsoft.PowerShell.PSConsoleReadLine]::KillLine()
    [Microsoft.PowerShell.PSConsoleReadLine]::Insert($fileContents)
}

Set-PSReadlineKeyHandler -Key 'Ctrl+k' -ScriptBlock {
    # Generate a unique temporary file name
    $tempFileName = [System.IO.Path]::GetTempFileName()
    # $tempFileName = "./testfile"
    $bufferState = $cursorState = $null
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref] $bufferState, [ref] $cursorState)
    # Write the buffer content to the temporary file
    $bufferState | Out-File -FilePath $tempFileName -Force
    # Remove the file extension from the temporary file name
    # $tempFileBaseName = [System.IO.Path]::GetFileNameWithoutExtension($tempFileName)
    # Run the CLI application with the temporary file
    Start-Process shai -ArgumentList "explain --model open-aigpt35turbo --edit-file $tempFileName" -Wait
    $fileContents = Get-Content -Raw -Path $tempFileName
    # # Remove the temporary file
    Remove-Item -Path $tempFileName -Force
    [Microsoft.PowerShell.PSConsoleReadLine]::BackwardKillLine()
    [Microsoft.PowerShell.PSConsoleReadLine]::KillLine()
    [Microsoft.PowerShell.PSConsoleReadLine]::Insert($fileContents)
}

