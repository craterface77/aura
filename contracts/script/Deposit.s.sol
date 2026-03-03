// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import {AuraL1Bridge} from "../src/AuraL1Bridge.sol";

contract DepositScript is Script {
    address constant BRIDGE_ADDRESS = 0x0FDEa090F5665b80C71a7E79E1B951Cb209d5B45;

    function run() external {
        uint256 deployerPrivateKey = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;

        vm.startBroadcast(deployerPrivateKey);

        AuraL1Bridge bridge = AuraL1Bridge(BRIDGE_ADDRESS);

        uint256 amount = 0.1 ether;

        console.log("Sending deposit...");
        bridge.deposit{value: amount}();
        console.log("Deposit sent! Check your Rust Ingestor logs.");

        vm.stopBroadcast();
    }
}
