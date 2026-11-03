class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.26.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.26.0/pytest-language-server-aarch64-apple-darwin"
      sha256 "4a4d125e7542ab1040790305af02f7ae0f7b827a50c373bcb61f49141dc5b19c"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.26.0/pytest-language-server-x86_64-apple-darwin"
      sha256 "37963d22adbaa1e35b27ffdb07ac0464a740eb8c95511f2260368e7ab4f2ba6a"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.26.0/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "f8254987bfcbda259cc7939d841a110651a27bf2034c576b18ce023671b0aa7b"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.26.0/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "dcfba40ace3784bcf2caf0a0a52af641304541d49019021a89fad30780a7bbd4"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
