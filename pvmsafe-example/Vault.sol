// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Vault {
    event Deposit(address indexed user, uint256 amount, uint256 shares);
    event Withdraw(address indexed user, uint256 amount, uint256 shares);
    error InsufficientShares();
    error ZeroAmount();

    function totalAssets() external view returns (uint256);
    function totalShares() external view returns (uint256);
    function sharesOf(address user) external view returns (uint256);

    function deposit(uint256 amount) external;
    function withdraw(uint256 shares) external;
}
