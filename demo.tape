# pytest-language-server Demo
# Showcasing LSP features in neovim
#
# To regenerate:
#   1. vhs demo.tape && mv demo.gif demo.mp4
#   2. ffmpeg -i demo.mp4 -vf "fps=20,scale=1200:-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse" -loop 0 demo.gif -y
#   3. rm demo.mp4
#
# Note: Using MP4 intermediate due to VHS GIF generation issues
# Requires: https://github.com/charmbracelet/vhs

Output demo.mp4

Set Shell "bash"
Set FontSize 12
Set Width 1200
Set Height 700
Set Padding 10
Set Framerate 20
Set Theme "Dracula"

Hide
Type "cd /Users/bellini/dev/pytest-language-server/tests/test_project" Enter
Show

# Title
Type 'echo "pytest-language-server Demo"' Enter
Sleep 1.5s
Type 'echo "1. Hover Documentation"' Enter
Sleep 1.5s

Type "nvim test_example.py" Enter
Sleep 2.5s

Hide
Type ":set nu" Enter
Show

Type "ggf(w"
Sleep 1s
Type "K"
Sleep 3.5s
Escape

Type "ZQ"
Sleep 300ms

Type 'echo "2. Go to Definition"' Enter
Sleep 1.5s

Type "nvim test_example.py" Enter
Sleep 2s

Hide
Type ":set nu" Enter
Show

Type "ggf(w"
Type "gd"
Sleep 2.5s
Ctrl+O

Type "ZQ"
Sleep 300ms

Type 'echo "3. Code Actions"' Enter
Sleep 1.5s

Type "nvim test_undeclared_example.py" Enter
Sleep 2s

Hide
Type ":set nu" Enter
Show

Type ":15" Enter
Type "0ww"
Sleep 1s

Type ":lua vim.lsp.buf.code_action()" Enter
Sleep 3s
Enter
Sleep 1s

Type ":9" Enter
Sleep 2s

Type "ZQ"

Type 'echo "github.com/bellini666/pytest-language-server"' Enter
Sleep 2s
