class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.19.2"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.19.2/pytest-language-server-aarch64-apple-darwin"
      sha256 "af0f6dada14b05cc8d091c2628fc799de5183eef1db3919fe2433119fd0ae987"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.19.2/pytest-language-server-x86_64-apple-darwin"
      sha256 "ae7c4ed8d58355bdd1cdf94ccd1e5dd2742e7404a32f7fdb29e3c623f1302f25"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.19.2/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "de8571a0caac67c919a1f8babf9168dc653b2fbf4bf31b91796adf0bddd5604f"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.19.2/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "c4be136ebbaea4c049d734a0916c252b5014dcd38d71164b2856332939f063c3"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
