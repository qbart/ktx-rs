// SPDX-FileCopyrightText: 2019-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//
// libktx_read.js factory (createKtxReadModule). Runs inside the factory scope after libktx_common.js.

return function createKtxReadModule(moduleArg) {
  return __ktxModule('libktx_read', moduleArg, __ktxLibktxPopulate);
};
