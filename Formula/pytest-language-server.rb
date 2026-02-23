class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.21.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.0/pytest-language-server-aarch64-apple-darwin"
      sha256 "d58daa60f1031454f08b3d27cc1b0e1c290becfa49d5a053f13e31e99cc7b2f5"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.0/pytest-language-server-x86_64-apple-darwin"
      sha256 "0c71321562201be79053183f31479fb31571a7ce6637283421e61beb8fe09e0a"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.0/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "c9e332f5d9ca33c7569133b2606ff9d39b00fb9dae20f44eec86d28cc9f9a2fa"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.0/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "e1abdf93a6cf01db5e2891df774d67c90d3e5a61ce7860ba3dbc7c21646ebb56"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
