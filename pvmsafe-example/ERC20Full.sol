// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface ERC20Full {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    error InsufficientBalance();
    error InsufficientAllowance();

    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
    function allowance(address owner, address spender) external view returns (uint256);

    function transfer(address to, uint256 amount) external;
    function approve(address spender, uint256 amount) external;
    function transferFrom(address from, address to, uint256 amount) external;
    function mint(address to, uint256 amount) external;
}
