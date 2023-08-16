$model = "open-aigpt35-turbo"

Set-PSReadlineKeyHandler -Key 'Alt+s' -ScriptBlock {
    # Generate a unique temporary file name
    $tempFileName = [System.IO.Path]::GetTempFileName()

    $bufferState = $cursorState = $null
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref] $bufferState, [ref] $cursorState)
    # Write the buffer content to the temporary file
    $bufferState | Out-File -FilePath $tempFileName -Force
    # Run the CLI application with the temporary file
    Start-Process shai -ArgumentList "ask --model $model --edit-file $tempFileName" -Wait
    $fileContents = Get-Content -Raw -Path $tempFileName
    # # Remove the temporary file
    Remove-Item -Path $tempFileName -Force
    [Microsoft.PowerShell.PSConsoleReadLine]::BackwardKillLine()
    [Microsoft.PowerShell.PSConsoleReadLine]::KillLine()
    [Microsoft.PowerShell.PSConsoleReadLine]::Insert($fileContents)
}

Set-PSReadlineKeyHandler -Key 'Alt+e' -ScriptBlock {
    # Generate a unique temporary file name
    $tempFileName = [System.IO.Path]::GetTempFileName()

    $bufferState = $cursorState = $null
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref] $bufferState, [ref] $cursorState)
    # Write the buffer content to the temporary file
    $bufferState | Out-File -FilePath $tempFileName -Force
    # Run the CLI application with the temporary file
    Start-Process shai -ArgumentList "explain --model $model --edit-file $tempFileName" -Wait
    $fileContents = Get-Content -Raw -Path $tempFileName
    # # Remove the temporary file
    Remove-Item -Path $tempFileName -Force
    [Microsoft.PowerShell.PSConsoleReadLine]::BackwardKillLine()
    [Microsoft.PowerShell.PSConsoleReadLine]::KillLine()
    [Microsoft.PowerShell.PSConsoleReadLine]::Insert($fileContents)
}

