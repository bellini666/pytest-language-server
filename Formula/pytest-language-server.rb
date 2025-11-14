class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.3.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.3.0/pytest_language_server-0.3.0-py3-none-macosx_11_0_arm64.whl"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_ARM64"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.3.0/pytest_language_server-0.3.0-py3-none-macosx_10_12_x86_64.whl"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_X86_64"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.3.0/pytest_language_server-0.3.0-py3-none-manylinux_2_17_aarch64.manylinux2014_aarch64.whl"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_LINUX_ARM64"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.3.0/pytest_language_server-0.3.0-py3-none-manylinux_2_17_x86_64.manylinux2014_x86_64.whl"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_LINUX_X86_64"
    end
  end

  depends_on "python@3.10" => :build

  def install
    # Install the wheel using pip
    system "pip3", "install", "--target=#{libexec}", cached_download

    # Create a wrapper script
    (bin/"pytest-language-server").write_env_script(
      "#{libexec}/bin/pytest-language-server",
      PYTHONPATH: libexec
    )
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version 2>&1", 1)
  end
end
