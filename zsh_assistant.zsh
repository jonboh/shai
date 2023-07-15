ask-assistant() {
    VISUAL="./target/debug/cli-ai-assistant ask --model open-aigpt35turbo --edit-file" zle edit-command-line
}
explain-assistant() {
    VISUAL="./target/debug/cli-ai-assistant explain --model open-aigpt35turbo --edit-file" zle edit-command-line
}
# Bind a key combination to trigger the custom widget
zle -N ask-assistant
zle -N explain-assistant
bindkey '^X' ask-assistant
bindkey '^G' explain-assistant
