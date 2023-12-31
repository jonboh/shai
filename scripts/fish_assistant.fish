set os "Linux" # use your distro to get more specific instructions
set shell "Fish"
set model "open-aigpt35-turbo"

function ask_assistant --description 'Edit command in Shai'
    set -l f (mktemp)
    if set -q f[1]
        mv $f $f.fish
        set f $f.fish
    else
        # We should never execute this block but better to be paranoid.
        set f /tmp/fish.(echo %self).fish
        touch $f
    end

    set -l p (commandline -C)
    commandline -b > $f
    shai ask --operating-system "$os" --shell "$shell" --model $model --edit-file $f

    commandline -r (cat $f)
    commandline -C $p
    command rm $f
end

function explain_assistant --description 'Let Shai explain this command'
    set -l f (mktemp)
    if set -q f[1]
        mv $f $f.fish
        set f $f.fish
    else
        # We should never execute this block but better to be paranoid.
        set f /tmp/fish.(echo %self).fish
        touch $f
    end

    set -l p (commandline -C)
    commandline -b > $f
    shai explain --operating-system "$os" --shell "$shell" --model $model --edit-file $f

    commandline -r (cat $f)
    commandline -C $p
    command rm $f
end

bind \es ask_assistant
bind \ee explain_assistant
