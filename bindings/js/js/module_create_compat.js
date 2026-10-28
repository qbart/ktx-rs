// SPDX-FileCopyrightText: 2019-2024 Khronos Group, Inc.
// SPDX-License-Identifier: Apache-2.0
//
// module_create_compat.js (--extern-post-js): old module create function name.

var LIBKTX
if (typeof createKtxReadModule === "function")
  LIBKTX = createKtxReadModule;
else
  LIBKTX = createKtxModule;
