model=open-aigpt35-turbo

_ask_assistant() {
    tmpf="$(mktemp)"
    printf '%s\n' "$READLINE_LINE" > "$tmpf"
    "shai" "ask" "--model" "$model" "--edit-file" "$tmpf"
    READLINE_LINE="$(<"$tmpf")"
    READLINE_POINT="${#READLINE_LINE}"
    rm -f "$tmpf"
}

_explain_assistant() {
    tmpf="$(mktemp)"
    printf '%s\n' "$READLINE_LINE" > "$tmpf"
    "shai" "explain" "--model" "$model""--edit-file" "$tmpf"
    READLINE_LINE="$(<"$tmpf")"
    READLINE_POINT="${#READLINE_LINE}"
    rm -f "$tmpf"
}

# Bind <C-x> to trigger the _assistant_complete function
bind -x '"\es":_ask_assistant'
bind -x '"\ee":_explain_assistant'
