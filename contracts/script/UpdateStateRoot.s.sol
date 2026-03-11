// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import {AuraL1Bridge} from "../src/AuraL1Bridge.sol";

/// Posts the current L2 state root to AuraL1Bridge on L1.
/// Called by the operator after a batch of L2 transactions.
///
/// Required env vars:
///   BRIDGE_CONTRACT — deployed bridge address
///   STATE_ROOT      — 0x-prefixed 32-byte hex state root from GET /state/root
contract UpdateStateRootScript is Script {
    function run() external {
        address bridge = vm.envAddress("BRIDGE_CONTRACT");
        bytes32 newRoot = vm.envBytes32("STATE_ROOT");
        uint256 privateKey = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;

        vm.startBroadcast(privateKey);

        AuraL1Bridge(bridge).updateStateRoot(newRoot);
        console.log("State root updated on L1:");
        console.logBytes32(newRoot);

        vm.stopBroadcast();
    }
}
