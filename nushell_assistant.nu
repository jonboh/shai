let model = "open-aigpt35-turbo"

$env.config.keybindings = ($env.config.keybindings | append {
            name: open_shai_generate
            modifier: alt
            keycode: char_s
            mode: [emacs, vi_normal, vi_insert]
            event: { send: executehostcommand
                     cmd: "commandline | save -f /tmp/reedline_buffer.nu;
                     shai ask --model $model --edit-file /tmp/reedline_buffer.nu;
                     commandline -r (cat /tmp/reedline_buffer.nu)"}
        }
)

$env.config.keybindings = ($env.config.keybindings | append {
            name: open_shai_explain
            modifier: alt
            keycode: char_e
            mode: [emacs, vi_normal, vi_insert]
            event: { send: executehostcommand
                     cmd: "commandline | save -f /tmp/reedline_buffer.nu;  
                           shai explain --model $model --edit-file /tmp/reedline_buffer.nu;
                           commandline -r (cat /tmp/reedline_buffer.nu)"}
        }
)
