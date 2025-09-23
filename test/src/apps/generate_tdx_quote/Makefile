# SPDX-License-Identifier: MPL-2.0

MAIN_MAKEFILE := $(firstword $(MAKEFILE_LIST))
INCLUDE_MAKEFILE := $(lastword $(MAKEFILE_LIST))
CUR_DIR := $(shell dirname $(realpath $(MAIN_MAKEFILE)))
CUR_DIR_NAME := $(shell basename $(realpath $(CUR_DIR)))
BUILD_DIR := $(CUR_DIR)/../../../build
OBJ_OUTPUT_DIR := $(BUILD_DIR)/initramfs/test/$(CUR_DIR_NAME)
DEP_OUTPUT_DIR := $(BUILD_DIR)/dep/$(CUR_DIR_NAME)
C_SRCS := $(wildcard *.c)
C_OBJS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(C_SRCS:%.c=%))
C_DEPS := $(addprefix $(DEP_OUTPUT_DIR)/,$(C_SRCS:%.c=%.d))
ASM_SRCS := $(wildcard *.S)
ASM_OBJS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(ASM_SRCS:%.S=%))
CC ?= gcc
C_FLAGS ?= -Wall -Werror
EXTRA_C_FLAGS := -DDEBUG

.PHONY: all

all: $(OBJ_OUTPUT_DIR) $(OBJ_OUTPUT_DIR)/generate_tdx_quote

$(OBJ_OUTPUT_DIR):
	@mkdir -p $(OBJ_OUTPUT_DIR)

$(OBJ_OUTPUT_DIR)/generate_tdx_quote: test_tdx_attest.c tdx_attest.c qgs_msg_lib.cpp
	@$(CC) $(C_FLAGS) $^ -o $@ $(EXTRA_C_FLAGS)
	@echo "CC <= $@"

$(OBJ_OUTPUT_DIR)/%: %.c
	@$(CC) $(C_FLAGS) $< -o $@ $(EXTRA_C_FLAGS)
	@echo "CC <= $@"