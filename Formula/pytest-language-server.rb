class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.17.3"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.3/pytest-language-server-aarch64-apple-darwin"
      sha256 "b4814a1092fde2848e050dc6725d1dfdc5b45eabcf9eeb619d8b0fe85972f310"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.3/pytest-language-server-x86_64-apple-darwin"
      sha256 "150337034deaa01ea14993de34fde9122e305edae1874a3ebf8ba8248032d1b7"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.3/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "cf7ca53510cc0c2875bb23103b492f78aeb3ebc6bbc6c2e737a0eb66ea192149"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.3/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "d102f5cd4bdd3603a76fa49182d9de577a22ec3ebfd1bac067cae09527da019b"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
