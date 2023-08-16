model="open-aigpt4"

ask-assistant() {
    VISUAL="shai ask --model $model --edit-file" zle edit-command-line
}
explain-assistant() {
    VISUAL="shai explain --model $model --edit-file" zle edit-command-line
}
# Bind a key combination to trigger the custom widget
zle -N ask-assistant
zle -N explain-assistant
bindkey '^[s' ask-assistant
bindkey '^[e' explain-assistant
