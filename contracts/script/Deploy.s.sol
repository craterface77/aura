// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Script} from "forge-std/Script.sol";
import {AuraL1Bridge} from "../src/AuraL1Bridge.sol";

contract DeployScript is Script {
    function run() public {
        vm.startBroadcast();

        AuraL1Bridge counter = new AuraL1Bridge();

        vm.stopBroadcast();
    }
}
