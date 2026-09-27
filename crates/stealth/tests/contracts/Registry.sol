// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

contract Registry {
    mapping(address => mapping(uint256 => bytes)) keys;

    function registerKeys(uint256 schemeId, bytes calldata stealthMetaAddress) external {
        keys[msg.sender][schemeId] = stealthMetaAddress;
    }

    function stealthMetaAddressOf(address registrant, uint256 schemeId)
        external
        view
        returns (bytes memory)
    {
        return keys[registrant][schemeId];
    }
}
