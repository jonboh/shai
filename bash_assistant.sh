_ask_assistant() {
    tmpf="$(mktemp)"
    printf '%s\n' "$READLINE_LINE" > "$tmpf"
    "./target/debug/shai-assistant" "ask" "--model" "open-aigpt35turbo" "--edit-file" "$tmpf"
    READLINE_LINE="$(<"$tmpf")"
    READLINE_POINT="${#READLINE_LINE}"
    rm -f "$tmpf"
}

_explain_assistant() {
    tmpf="$(mktemp)"
    printf '%s\n' "$READLINE_LINE" > "$tmpf"
    "./target/debug/shai-assistant" "explain" "--model" "open-aigpt35turbo" "--edit-file" "$tmpf"
    READLINE_LINE="$(<"$tmpf")"
    READLINE_POINT="${#READLINE_LINE}"
    rm -f "$tmpf"
}

# Bind <C-x> to trigger the _assistant_complete function
bind -x '"\C-x":_ask_assistant'
bind -x '"\C-k":_explain_assistant'
