// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import {AuraL1Bridge} from "../src/AuraL1Bridge.sol";

/// Reads proof data from environment variables (set by the shell wrapper)
/// and submits a Merkle-proof withdrawal to AuraL1Bridge.
///
/// Required env vars:
///   BRIDGE_CONTRACT  — deployed bridge address
///   WITHDRAW_AMOUNT  — wei to withdraw
///   L2_BALANCE       — full L2 balance (as in the SMT leaf)
///   LEAF_INDEX       — uint256 leaf position in the SMT
///   SIBLINGS         — 32 hex-encoded bytes32 values, comma-separated
///   PRIVATE_KEY      — caller private key
contract WithdrawScript is Script {
    function run() external {
        address bridge = vm.envAddress("BRIDGE_CONTRACT");
        uint256 amount = vm.envUint("WITHDRAW_AMOUNT");
        uint256 l2Balance = vm.envUint("L2_BALANCE");
        uint256 leafIndex = vm.envUint("LEAF_INDEX");
        uint256 privateKey = vm.envUint("PRIVATE_KEY");

        // Parse SIBLINGS env var: 32 comma-separated 0x-prefixed hex strings.
        bytes32[32] memory siblings;
        string memory siblingsRaw = vm.envString("SIBLINGS");
        string[] memory parts = vm.split(siblingsRaw, ",");
        require(parts.length == 32, "SIBLINGS must have exactly 32 entries");
        for (uint256 i = 0; i < 32; i++) {
            siblings[i] = vm.parseBytes32(parts[i]);
        }

        vm.startBroadcast(privateKey);

        AuraL1Bridge(bridge).withdraw(amount, l2Balance, leafIndex, siblings);
        console.log("Withdrew", amount, "wei from bridge", bridge);

        vm.stopBroadcast();
    }
}
