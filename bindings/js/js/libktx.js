// SPDX-FileCopyrightText: 2019-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//
// libktx.js factory (createKtxModule). Runs inside the factory scope after libktx_common.js.

return function createKtxModule(moduleArg) {
  return __ktxModule('libktx', moduleArg, __ktxLibktxPopulate);
};
