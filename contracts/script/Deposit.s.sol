// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import {AuraL1Bridge} from "../src/AuraL1Bridge.sol";

contract DepositScript is Script {
    function run() external {
        address bridge = vm.envAddress("BRIDGE_CONTRACT");
        uint256 deployerPrivateKey = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;

        vm.startBroadcast(deployerPrivateKey);

        AuraL1Bridge(bridge).deposit{value: 1 ether}();
        console.log("Deposit of 1 ETH sent to bridge:", bridge);

        vm.stopBroadcast();
    }
}
