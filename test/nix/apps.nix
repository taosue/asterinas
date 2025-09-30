{ lib, stdenv, fetchFromGitHub, fetchurl, hostPlatform, glibc, libnl, }: rec {

  DCAP_VERSION = "DCAP_1.23";
  DCAP_URL_PREFIX = "https://github.com/intel/SGXDataCenterAttestationPrimitives/raw/${DCAP_VERSION}/QuoteGeneration/quote_wrapper";

  tdx_attest_c = fetchurl {
    url = "${DCAP_URL_PREFIX}/tdx_attest/tdx_attest.c";
    sha256 = "08aijjx7jnmswimv4dhfwgbb0inwl0xg9hry37zy8k4wln6dys27";
  };
  tdx_attest_h = fetchurl {
    url = "${DCAP_URL_PREFIX}/tdx_attest/tdx_attest.h";
    sha256 = "0zsljf3gm9x0rp6dyin039akaf6lwf9fj0d6dskjzmlnsfzhqhmb";
  };
  test_tdx_attest_c = fetchurl {
    url = "${DCAP_URL_PREFIX}/tdx_attest/test_tdx_attest.c";
    sha256 = "1l7gx7wd2462ghwvf3i17kp7phq0sgyb22rpx568zlha48jqp9sc";
  };
  qgs_msg_lib_cpp = fetchurl {
    url = "${DCAP_URL_PREFIX}/qgs_msg_lib/qgs_msg_lib.cpp";
    sha256 = "0ffnmy8vg5yn12d9mz1zjdlfg98i9k112kyybr1fnm5yh1rdcnys";
  };
  qgs_msg_lib_h = fetchurl {
    url = "${DCAP_URL_PREFIX}/qgs_msg_lib/inc/qgs_msg_lib.h";
    sha256 = "092dvr5qbrwk707s0jwgqz79cw0dimp1n2qqkl9v6dik8l9fgfa6";
  };

  mongoose_src = fetchFromGitHub {
    owner = "cesanta";
    repo = "mongoose";
    rev = "7.13";
    sha256 = "sha256-9XHUE8SVOG/X7SIB52C8EImPx4XZ7B/5Ojwmb0PkiuI";
  };

  package = stdenv.mkDerivation {
    pname = "apps";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./../src;
      fileset = ./../src/apps;
    };

    MONGOOSE_DIR = "${mongoose_src}";

    HOST_PLATFORM = "${hostPlatform.system}";
    CC = "${stdenv.cc.targetPrefix}cc";
    C_FLAGS = "-I${libnl.dev}/include/libnl3";
    # FIXME: Excluding `glibc` allows the build to succeed, but causes some tests to fail.
    buildInputs = [ glibc glibc.static libnl ];
    buildCommand = ''
      BUILD_DIR=$(mktemp -d)
      mkdir -p $BUILD_DIR
      cp -r $src/apps $BUILD_DIR/

      chmod +w $BUILD_DIR/apps/generate_tdx_quote
      cp ${tdx_attest_c} $BUILD_DIR/apps/generate_tdx_quote/tdx_attest.c
      cp ${tdx_attest_h} $BUILD_DIR/apps/generate_tdx_quote/tdx_attest.h
      cp ${test_tdx_attest_c} $BUILD_DIR/apps/generate_tdx_quote/test_tdx_attest.c
      cp ${qgs_msg_lib_cpp} $BUILD_DIR/apps/generate_tdx_quote/qgs_msg_lib.cpp
      cp ${qgs_msg_lib_h} $BUILD_DIR/apps/generate_tdx_quote/qgs_msg_lib.h

      pushd $BUILD_DIR
      make --no-print-directory -C apps
      popd

      mkdir -p $out
      mv build/initramfs/test/* $out/
    '';
  };
}
