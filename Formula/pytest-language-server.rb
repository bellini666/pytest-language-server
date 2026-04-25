class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.22.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.1/pytest-language-server-aarch64-apple-darwin"
      sha256 "3c57ed8634122df68991ff47e3c73b0c5d8b6c0f1d5d557b4da114a127ab1a15"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.1/pytest-language-server-x86_64-apple-darwin"
      sha256 "495c36b4271d467b6cde1dfe02ee9a49233af957c1fe5759d6426d2e2ca26c87"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.1/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "a55b2f3d6899d1cef65379fec6d5b019a8f3a41cb808af9508987c8843dd0c42"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.1/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "778ded437e4918b16c2a975aea01c5c0d735acdcd252a7738fca7e924983325a"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
