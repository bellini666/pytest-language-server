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
Set Width 800
Set Height 400
Set Padding 10
Set Framerate 30
Set Theme "Dracula"

Hide
Type "cd /Users/bellini/dev/pytest-language-server/tests/test_project" Enter
Show

# Title
Type 'echo "🐍 pytest-language-server Demo ⚡"' Enter
Sleep 1.5s
Type 'echo "📖 1. Hover Documentation & Go to Definition"' Enter
Sleep 1.5s

Type "nvim test_example.py" Enter
Sleep 2.5s

Hide
Type ":set nu" Enter
Show

# Hover on sample_fixture (position cursor on the fixture name)
Type "ggf(w"
Sleep 1s
Type "K"
Sleep 3s

# Go to definition (this will close hover and jump)
Type "gd"
Sleep 2.5s
Ctrl+O
Sleep 500ms

Type "ZQ"
Sleep 300ms

Type 'echo "✨ 2. Code Completion"' Enter
Sleep 1.5s

Type "nvim test_example.py" Enter
Sleep 2.5s

Hide
Type ":set nu" Enter
Show

# Navigate to first test function and go to end of parameters
Type "gg"
Sleep 300ms
Type "f)"
Sleep 300ms
Type "i, "
Sleep 500ms

# Trigger completion
Ctrl+X
Ctrl+O
Sleep 2.5s

# Navigate down in completion menu
Down
Sleep 500ms
Down
Sleep 500ms
Enter
Sleep 1.5s

# Exit without saving
Escape
Sleep 300ms
Type ":q!" Enter
Sleep 300ms

Type 'echo "🔧 3. Code Actions (Quick Fixes)"' Enter
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
Sleep 300ms

Type 'echo "✨ github.com/bellini666/pytest-language-server ✨"' Enter
Sleep 2s
