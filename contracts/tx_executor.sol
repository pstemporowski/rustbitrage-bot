// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IUniswapV2Router02} from "path/to/IUniswapV2Router02.sol";
import {ILendingPool} from "path/to/ILendingPool.sol";

contract ArbitrageExecutor {
    address public owner;
    ILendingPool public lendingPool;
    IUniswapV2Router02 public router;

    constructor(address _lendingPool, address _router) {
        owner = msg.sender;
        lendingPool = ILendingPool(_lendingPool);
        router = IUniswapV2Router02(_router);
    }

    function executeArbitrage(
        uint256 amount,
        address[] calldata path
    ) external {
        require(msg.sender == owner, "Only owner can call");

        // Initiate flash loan
        address asset = path[0];
        bytes memory data = abi.encode(amount, path);
        lendingPool.flashLoan(address(this), asset, amount, data);
    }

    // Aave flash loan callback
    function executeOperation(
        address asset,
        uint256 amount,
        uint256 premium,
        address initiator,
        bytes calldata params
    ) external returns (bool) {
        require(
            msg.sender == address(lendingPool),
            "Caller must be lending pool"
        );
        require(initiator == address(this), "Only this contract can initiate");

        (uint256 loanAmount, address[] memory path) = abi.decode(
            params,
            (uint256, address[])
        );

        // Perform arbitrage
        uint256 amountOut = loanAmount;
        for (uint256 i = 0; i < path.length - 1; i++) {
            address fromToken = path[i];
            address toToken = path[i + 1];

            // Approve router to spend fromToken
            IERC20(fromToken).approve(address(router), amountOut);

            address[] memory swapPath = new address[](2);
            swapPath[0] = fromToken;
            swapPath[1] = toToken;

            uint256[] memory amounts = router.swapExactTokensForTokens(
                amountOut,
                0,
                swapPath,
                address(this),
                block.timestamp
            );

            amountOut = amounts[1];
        }

        // Calculate total debt to repay
        uint256 totalDebt = amount + premium;

        // Approve lending pool to pull the owed amount
        IERC20(asset).approve(address(lendingPool), totalDebt);

        // Keep the profit
        uint256 profit = amountOut - totalDebt;
        if (profit > 0) {
            IERC20(asset).transfer(owner, profit);
        }

        return true;
    }
}
